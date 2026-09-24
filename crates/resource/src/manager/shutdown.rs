//! Graceful shutdown machinery for the [`Manager`].
//!
//! Phases:
//!
//! 1. **SIGNAL** — cancel the manager token, rejecting new acquires.
//! 2. **RETIRE AND DRAIN** — relinquish retained and idle roots while waiting for handles, honouring
//!    [`DrainTimeoutPolicy`].
//! 3. **FINALIZE** — settle retirement and wait for release-queue workers to exit.
//!
//! Errors are typed [`ShutdownError`] variants; the previous behaviour of
//! silently force-clearing the registry on drain timeout is now opt-in
//! through [`DrainTimeoutPolicy::Force`].

use std::{
    sync::{Arc, atomic::Ordering as AtomicOrdering},
    time::Duration,
};

use tokio::sync::Notify;

use crate::{
    events::{ResourceEvent, RetirementOrigin},
    manager::{DrainTimeoutPolicy, Manager, ShutdownConfig},
    release_queue::{ReleaseQueue, ReleaseQueueHandle},
};

use super::retirement::RetirementFailure;
use super::shutdown_session::{DrainFailure, ShutdownSession, ShutdownState};

fn shutdown_state_name(state: &ShutdownState) -> &'static str {
    match state {
        ShutdownState::Open => "open",
        ShutdownState::Draining(_) => "draining",
        ShutdownState::Finishing(_) => "finishing",
        ShutdownState::Finished => "finished",
    }
}

async fn await_terminal_task(state: &mut ShutdownState) -> Result<ShutdownReport, ShutdownError> {
    let ShutdownState::Finishing(handle) = state else {
        tracing::error!(
            shutdown.state = shutdown_state_name(state),
            shutdown.expected_state = "finishing",
            "resource manager shutdown state invariant violated before awaiting terminal task"
        );
        return Err(ShutdownError::RetirementSupervisorFailed);
    };
    let result = match handle.await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(
                cancelled = error.is_cancelled(),
                panicked = error.is_panic(),
                "manager-owned terminal shutdown task failed"
            );
            Err(ShutdownError::RetirementSupervisorFailed)
        },
    };
    *state = ShutdownState::Finished;
    result
}

/// Waits until `tracker`'s counter reaches `0` or `timeout` elapses.
///
/// Shared by the manager-wide shutdown session (drains
/// `Manager::drain_tracker` for `graceful_shutdown`) and the per-resource
/// drain in `Manager::revoke_resolved` (drains a single
/// [`ManagedResource`](crate::ManagedResource)'s own
/// in-flight tracker). This helper is the single source of the subtle
/// lost-wakeup ordering — it is written **once** here rather than
/// duplicated per call site (a structural guarantee, not a discipline one);
/// the [`manager`](crate::manager) module docs delegate the ordering recipe
/// to this function.
///
/// The loop uses a `register-then-check` ordering to avoid the classic
/// `Notify::notify_waiters` lost-wakeup:
///
/// 1. Construct + pin + `enable()` a fresh `Notified` future. `enable()`
///    registers this waiter on the `Notify` queue without an `.await`, so
///    any subsequent `notify_waiters()` (fired when a handle's `Drop`
///    decrements the counter from `1 → 0`) reaches us.
/// 2. Re-check the counter. If it hit `0` between the outer check and our
///    registration, return now — the wakeup is already consumed.
/// 3. Only then await the `Notified` future.
///
/// Returns `Ok(())` once drained, or `Err(outstanding)` with the snapshot of
/// the counter at the moment the timer fired.
pub(crate) async fn wait_for_tracker_drain(
    tracker: &Arc<(std::sync::atomic::AtomicU64, Notify)>,
    timeout: Duration,
) -> Result<(), u64> {
    let active = tracker.0.load(AtomicOrdering::Acquire);
    if active == 0 {
        return Ok(());
    }

    tracing::debug!(active_handles = active, "waiting for handles to drain");
    let drained = tokio::time::timeout(timeout, async {
        loop {
            // Pre-register this waiter BEFORE re-checking the counter.
            let notified = tracker.1.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            // Re-check after registration. If the last handle dropped while
            // we were between the outer check and `enable()`, the counter is
            // now 0 and we would otherwise wait on a notification that has
            // already fired.
            if tracker.0.load(AtomicOrdering::Acquire) == 0 {
                return;
            }

            notified.await;

            if tracker.0.load(AtomicOrdering::Acquire) == 0 {
                return;
            }
        }
    })
    .await;

    if drained.is_err() {
        let outstanding = tracker.0.load(AtomicOrdering::Acquire);
        tracing::warn!(
            outstanding,
            "resource manager: drain timeout expired with handles still active"
        );
        return Err(outstanding);
    }
    Ok(())
}

/// Structured result of a successful (or forced-through) graceful shutdown.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// How many `ResourceGuard`s were still outstanding when the drain
    /// phase finished. Zero on the happy path. Nonzero only when the
    /// caller explicitly opted into [`DrainTimeoutPolicy::Force`].
    pub outstanding_handles_after_drain: u64,
    /// Whether all rows were removed from the registry and queued for cleanup.
    pub registry_cleared: bool,
    /// Whether every release-queue worker completed within its budget.
    /// False after Force with outstanding handles: cleanup remains open.
    /// Includes nested and rescue dispatchers, but does not prove provider success.
    pub release_queue_drained: bool,
    /// Cumulative queue-lifetime count of release tasks abandoned before
    /// their futures completed, including untouched batch members. Late Force-policy
    /// releases may increment the counter after this snapshot. Zero does not
    /// prove complete cleanup: a `Future<Output = ()>` can hide provider errors.
    pub dropped_release_tasks: usize,
}

/// Errors returned by [`Manager::graceful_shutdown`].
///
/// Each variant corresponds to a failure mode that was previously silently
/// absorbed by the old infallible signature. A timeout during drain, for
/// example, used to be a `tracing::warn!` and a forced `registry.clear()`;
/// it is now a typed error that the caller must handle.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum ShutdownError {
    /// Another caller is actively driving shutdown, or shutdown already completed.
    #[error("graceful shutdown already in progress")]
    AlreadyShuttingDown,

    /// The drain phase did not finish within `drain_timeout` and the
    /// policy was [`DrainTimeoutPolicy::Abort`]. The registry was **not**
    /// cleared and any outstanding handles remain valid, but every
    /// registered resource is transitioned to
    /// [`ResourcePhase::Failed`](crate::state::ResourcePhase::Failed) so
    /// subsequent acquires fail fast and `health_check` reflects the
    /// post-abort reality.
    #[error(
        "drain timeout expired with {outstanding} handle(s) still active; registry was NOT cleared (policy=Abort)"
    )]
    DrainTimeout {
        /// Snapshot of the drain-tracker counter at the moment the timeout
        /// fired.
        outstanding: u64,
    },

    /// Snapshot publication, retirement or release-worker joining exceeded the
    /// original shutdown envelope or the terminal `release_queue_timeout` budget.
    #[error(
        "resource shutdown did not complete within its configured deadline (cleanup budget: {timeout:?})"
    )]
    ReleaseQueueTimeout {
        /// The configured cleanup budget; the original shutdown envelope may
        /// leave less time available to the terminal stage.
        timeout: Duration,
    },

    /// A release worker failed to join. Other workers were aborted and joined;
    /// the error deliberately excludes third-party panic payloads.
    #[error("release queue worker failed")]
    ReleaseQueueWorkerFailed,

    /// A manager-owned publication, retirement-supervisor, or terminal task
    /// stopped before completing its shutdown work.
    #[error("resource shutdown task failed before completing terminal work")]
    RetirementSupervisorFailed,

    /// A row's terminal cleanup failed. Other rows still receive cleanup.
    #[error("resource terminal cleanup failed for {key}")]
    ResourceTeardownFailed {
        /// Resource whose cleanup failed.
        key: nebula_core::ResourceKey,
        /// Typed provider or framework failure.
        #[source]
        source: crate::Error,
    },
}

impl std::fmt::Debug for ShutdownError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyShuttingDown => formatter.write_str("AlreadyShuttingDown"),
            Self::DrainTimeout { outstanding } => formatter
                .debug_struct("DrainTimeout")
                .field("outstanding", outstanding)
                .finish(),
            Self::ReleaseQueueTimeout { timeout } => formatter
                .debug_struct("ReleaseQueueTimeout")
                .field("timeout", timeout)
                .finish(),
            Self::ReleaseQueueWorkerFailed => formatter.write_str("ReleaseQueueWorkerFailed"),
            Self::RetirementSupervisorFailed => formatter.write_str("RetirementSupervisorFailed"),
            Self::ResourceTeardownFailed { key, source } => formatter
                .debug_struct("ResourceTeardownFailed")
                .field("key", key)
                .field("source_kind", source.kind())
                .finish_non_exhaustive(),
        }
    }
}

/// Counts the ownership of a terminal job, including an unpolled factory.
pub(super) struct RetirementSettlement(Arc<(std::sync::atomic::AtomicU64, Notify)>);

impl RetirementSettlement {
    pub(super) fn new(tracker: Arc<(std::sync::atomic::AtomicU64, Notify)>) -> Self {
        tracker.0.fetch_add(1, AtomicOrdering::AcqRel);
        Self(tracker)
    }
}

impl Drop for RetirementSettlement {
    fn drop(&mut self) {
        if self.0.0.fetch_sub(1, AtomicOrdering::AcqRel) == 1 {
            self.0.1.notify_waiters();
        }
    }
}

/// Converts the first typed failure after the supervisor settles all siblings.
fn retirement_failure(failure: RetirementFailure) -> ShutdownError {
    tracing::warn!(
        resource.key = %failure.key,
        retirement.origin = ?failure.origin,
        error.kind = ?failure.source.kind(),
        "resource manager returning the first previously observed retirement failure"
    );
    ShutdownError::ResourceTeardownFailed {
        key: failure.key,
        source: failure.source,
    }
}

/// Owns every terminal value after the registry snapshot. The public
/// shutdown future only awaits this stored task, so cancelling that waiter
/// cannot drop unpublished rows or detach supervisor/release-worker handles.
#[tracing::instrument(skip_all, fields(outstanding_handles = outstanding_after_drain))]
async fn run_terminal_shutdown(
    session: ShutdownSession,
    retirement_supervisor: Arc<super::retirement::RetirementSupervisor>,
    retirement_tracker: Arc<(std::sync::atomic::AtomicU64, Notify)>,
    drain_tracker: Arc<(std::sync::atomic::AtomicU64, Notify)>,
    release_queue: Arc<ReleaseQueue>,
    release_queue_handle: Arc<tokio::sync::Mutex<Option<ReleaseQueueHandle>>>,
    outstanding_after_drain: u64,
) -> Result<ShutdownReport, ShutdownError> {
    let started = tokio::time::Instant::now();
    let budget = session.terminal_budget();
    let (config, mut terminal_error) = session.settle_publication(budget).await;

    if outstanding_after_drain > 0 && terminal_error.is_none() {
        tracing::warn!(
            outstanding = outstanding_after_drain,
            "resource manager: forced retirement incomplete; late cleanup remains open"
        );
        return Ok(ShutdownReport {
            outstanding_handles_after_drain: outstanding_after_drain,
            registry_cleared: true,
            release_queue_drained: false,
            dropped_release_tasks: release_queue.dropped_count(),
        });
    }

    if terminal_error.is_none() {
        let remaining = budget.saturating_sub(started.elapsed());
        terminal_error = match tokio::time::timeout(remaining, retirement_supervisor.seal()).await {
            Ok(Ok(())) => None,
            Ok(Err(error)) => {
                tracing::error!(
                    shutdown.phase = "retirement_seal",
                    supervisor.state = "seal_rejected",
                    error.kind = ?error.kind(),
                    "retirement supervisor rejected the terminal seal command"
                );
                Some(ShutdownError::RetirementSupervisorFailed)
            },
            Err(_) => Some(ShutdownError::ReleaseQueueTimeout {
                timeout: config.release_queue_timeout,
            }),
        };
    }

    let remaining = budget.saturating_sub(started.elapsed());
    let supervisor_error = if let Some(handle) = retirement_supervisor.take_handle() {
        match handle.join_bounded(remaining).await {
            Ok(first_failure) => first_failure.map(retirement_failure),
            Err(super::retirement::RetirementSupervisorJoinError::TimedOut) => {
                Some(ShutdownError::ReleaseQueueTimeout {
                    timeout: config.release_queue_timeout,
                })
            },
            Err(super::retirement::RetirementSupervisorJoinError::Failed) => {
                tracing::error!(
                    shutdown.phase = "retirement_join",
                    supervisor.state = "task_failed",
                    "retirement supervisor task failed before terminal settlement"
                );
                Some(ShutdownError::RetirementSupervisorFailed)
            },
        }
    } else {
        tracing::error!(
            shutdown.phase = "retirement_join",
            supervisor.state = "handle_missing",
            "retirement supervisor handle was already consumed before terminal settlement"
        );
        Some(ShutdownError::RetirementSupervisorFailed)
    };
    if terminal_error.is_none() {
        terminal_error = supervisor_error;
    }

    // Publication/finalization failure cannot revoke late guards' release path.
    // The publisher and supervisor are settled, but workers remain manager-owned.
    let outstanding = drain_tracker.0.load(AtomicOrdering::Acquire);
    if outstanding > 0 {
        tracing::warn!(
            outstanding,
            "resource manager: shutdown failed; late cleanup remains open"
        );
        return Err(terminal_error.unwrap_or(ShutdownError::RetirementSupervisorFailed));
    }

    let remaining = budget.saturating_sub(started.elapsed());
    if wait_for_tracker_drain(&retirement_tracker, remaining)
        .await
        .is_err()
        && terminal_error.is_none()
    {
        terminal_error = Some(ShutdownError::ReleaseQueueTimeout {
            timeout: config.release_queue_timeout,
        });
    }

    // All outside-queue coordinators are joined (or abort-joined), so no
    // task can publish terminal work after queue admission is sealed.
    release_queue.close();
    let handle = release_queue_handle.lock().await.take();
    if let Some(handle) = handle {
        let remaining = budget.saturating_sub(started.elapsed());
        let worker_error = ReleaseQueue::shutdown_bounded(handle, remaining)
            .await
            .err()
            .map(|error| match error {
                ShutdownError::ReleaseQueueTimeout { .. } => ShutdownError::ReleaseQueueTimeout {
                    timeout: config.release_queue_timeout,
                },
                other => other,
            });
        if terminal_error.is_none() {
            terminal_error = worker_error;
        }
    }
    if let Some(error) = terminal_error {
        return Err(error);
    }

    let dropped_release_tasks = release_queue.dropped_count();
    tracing::info!(dropped_release_tasks, "resource manager: shutdown complete");
    Ok(ShutdownReport {
        outstanding_handles_after_drain: 0,
        registry_cleared: true,
        release_queue_drained: true,
        dropped_release_tasks,
    })
}

impl Manager {
    /// Triggers graceful shutdown with drain and cleanup.
    ///
    /// 1. **Signal** — cancels the token so new acquires are rejected.
    /// 2. **Retire and drain** — fences stores and relinquishes retained/idle roots while
    ///    waiting up to [`ShutdownConfig::drain_timeout`] for in-flight handles.
    ///    A parent instance can therefore release its same-manager child guards during drain.
    ///    Shared instances remain usable until their final owning lease releases.
    /// 3. **Finalize** — clears the diagnostic registry index and settles retirement.
    /// 4. **Await workers** — waits for the release queue workers to finish processing remaining
    ///    tasks.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_resource::manager::{Manager, ShutdownConfig};
    /// # use std::time::Duration;
    /// # async fn example() {
    /// let manager = Manager::new();
    /// manager
    ///     .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_secs(5)))
    ///     .await
    ///     .expect("graceful shutdown should succeed");
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ShutdownError::AlreadyShuttingDown`] if another caller is actively
    ///   driving shutdown, or terminal shutdown already completed.
    /// - [`ShutdownError::DrainTimeout`] if in-flight handles do not release
    ///   within [`ShutdownConfig::drain_timeout`] and
    ///   [`ShutdownConfig::on_drain_timeout`] is
    ///   [`DrainTimeoutPolicy::Abort`] (the registry is left un-cleared so a
    ///   caller can inspect what is still outstanding).
    /// - [`ShutdownError::ReleaseQueueTimeout`] if snapshot publication,
    ///   retirement, or release-worker joining exceeds the original shutdown
    ///   envelope or the terminal [`ShutdownConfig::release_queue_timeout`] budget.
    /// - [`ShutdownError::ReleaseQueueWorkerFailed`] if a worker failed to join.
    /// - [`ShutdownError::RetirementSupervisorFailed`] if a manager-owned
    ///   publication, retirement-supervisor, or terminal task fails.
    /// - [`ShutdownError::ResourceTeardownFailed`] if a current row's terminal cleanup failed.
    ///
    /// With Force and outstanding handles, returns an incomplete report without
    /// closing cleanup. Late releases can finish while the manager stays alive.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe and resumable. The shutdown flag and cancellation token are
    /// set synchronously before the first await. During DRAIN, cancellation
    /// preserves the registry and a later call resumes using the first caller's
    /// policy and start time. A single manager-owned publisher owns the fenced snapshot
    /// before the first await and publishes it while drain runs. Publication failure
    /// enters terminal finalization; an Abort drain timeout preserves the session.
    /// Neither cancellation nor retry restarts either budget. Finalization ends by
    /// the earlier of its own cleanup budget and the original drain-plus-cleanup envelope.
    /// A manager-owned terminal task owns publisher/supervisor/worker joining; cancelling a
    /// caller only stops waiting for that task. A later call awaits the same
    /// terminal result. Concurrent drivers still fail fast with
    /// [`ShutdownError::AlreadyShuttingDown`].
    pub async fn graceful_shutdown(
        &self,
        config: ShutdownConfig,
    ) -> Result<ShutdownReport, ShutdownError> {
        // The guard is deliberately held across awaits. Caller cancellation
        // releases only this driver lease; all terminal owners live either in
        // `Manager` or its stored terminal task. A concurrent driver fails fast.
        let mut state = self
            .shutdown_state
            .try_lock()
            .map_err(|_| ShutdownError::AlreadyShuttingDown)?;
        if matches!(*state, ShutdownState::Finished) {
            return Err(ShutdownError::AlreadyShuttingDown);
        }
        if matches!(*state, ShutdownState::Finishing(_)) {
            return await_terminal_task(&mut state).await;
        }

        if matches!(*state, ShutdownState::Open) {
            let started = tokio::time::Instant::now();
            let _admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self
                .shutting_down
                .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
                .is_err()
            {
                return Err(ShutdownError::AlreadyShuttingDown);
            }

            tracing::info!("resource manager: starting graceful shutdown");

            // Reject acquires while preserving cleanup for guards released during drain.
            self.cancel.cancel();

            let retirements = self
                .registry
                .all_managed()
                .into_iter()
                .map(|managed| {
                    managed.set_phase(crate::state::ResourcePhase::Draining);
                    self.prepare_retirement(managed, RetirementOrigin::Shutdown)
                })
                .collect();
            *state = ShutdownState::Draining(ShutdownSession::start(
                config,
                started,
                retirements,
                Arc::clone(&self.retirement_supervisor),
            ));
        }

        let ShutdownState::Draining(session) = &mut *state else {
            tracing::error!(
                shutdown.state = shutdown_state_name(&state),
                shutdown.expected_state = "draining",
                "resource manager shutdown state invariant violated before drain"
            );
            return Err(ShutdownError::RetirementSupervisorFailed);
        };
        let outstanding_after_drain = match session.drain(&self.drain_tracker).await {
            Ok(()) => 0,
            Err(DrainFailure::Publication) => self.drain_tracker.0.load(AtomicOrdering::Acquire),
            Err(DrainFailure::TimedOut { outstanding }) => match session.config.on_drain_timeout {
                DrainTimeoutPolicy::Abort => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Abort — \
                         registry preserved, marking all resources Failed, \
                         returning DrainTimeout"
                    );
                    let err = ShutdownError::DrainTimeout { outstanding };
                    self.set_phase_all_failed(&err);
                    // Keep the original snapshot, publisher and deadline. A later
                    // driver can observe a zero counter even after drain expiry.
                    return Err(err);
                },
                DrainTimeoutPolicy::Force => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Force — \
                         clearing registry anyway"
                    );
                    outstanding
                },
            },
        };

        {
            let _admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.set_phase_all(crate::state::ResourcePhase::ShuttingDown);
            self.registry.clear();
            #[cfg(feature = "rotation")]
            for index in self.attached_rotation_indexes() {
                index.clear_for_manager_shutdown();
            }
        }
        let previous_state = std::mem::replace(&mut *state, ShutdownState::Finished);
        let session = match previous_state {
            ShutdownState::Draining(session) => session,
            unexpected_state => {
                tracing::error!(
                    shutdown.state = shutdown_state_name(&unexpected_state),
                    shutdown.expected_state = "draining",
                    "resource manager shutdown state invariant violated before terminal ownership transfer"
                );
                return Err(ShutdownError::RetirementSupervisorFailed);
            },
        };
        *state = ShutdownState::Finishing(tokio::spawn(run_terminal_shutdown(
            session,
            Arc::clone(&self.retirement_supervisor),
            Arc::clone(&self.retirement_tracker),
            Arc::clone(&self.drain_tracker),
            Arc::clone(&self.release_queue),
            Arc::clone(&self.release_queue_handle),
            outstanding_after_drain,
        )));
        await_terminal_task(&mut state).await
    }

    /// Drives every registered resource to the given lifecycle phase.
    ///
    /// Type-erased bulk update used during graceful shutdown so that
    /// `health_check` returns the correct phase while the drain/cleanup
    /// is in flight.
    pub(super) fn set_phase_all(&self, phase: crate::state::ResourcePhase) {
        for managed in self.registry.all_managed() {
            managed.set_phase(phase);
        }
    }

    /// Marks every registered resource as `Failed` with the supplied
    /// shutdown error and emits a per-resource
    /// [`ResourceEvent::HealthChanged`] with `healthy: false`.
    ///
    /// Used by the [`DrainTimeoutPolicy::Abort`] branch so the registry's
    /// recorded phase agrees with the `Err(DrainTimeout)` observed by the
    /// caller. Without this, `health_check` would report `Ready` while
    /// `lookup` rejected acquires via the cancel token — the exact "phase
    /// corruption" failure mode this method closes.
    ///
    /// Emission is best-effort (no live subscribers is a no-op), matching the
    /// rest of the manager's event-emission policy via the `emit` helper.
    pub(super) fn set_phase_all_failed(&self, error: &ShutdownError) {
        let reason = error.to_string();
        for managed in self.registry.all_managed() {
            // A drain-abort leaves the resource permanently bankrupt — the
            // manager will reject every subsequent acquire until it is
            // re-registered, so this is a non-retryable `Permanent` failure.
            managed.set_failed(crate::error::ErrorKind::Permanent, &reason);
            self.emit(ResourceEvent::HealthChanged {
                key: managed.resource_key(),
                healthy: false,
            });
        }
    }

    /// Waits until all active manager-wide `ResourceGuard`s are dropped or
    /// `timeout` expires (the `graceful_shutdown` drain).
    ///
    /// Thin typed wrapper over [`wait_for_tracker_drain`] against
    /// `Manager::drain_tracker`; the lost-wakeup-safe ordering and its
    /// rationale live on that shared helper (the per-resource revoke drain
    /// reuses the same helper against a single resource's tracker).
    #[cfg(test)]
    pub(super) async fn wait_for_drain(&self, timeout: Duration) -> Result<(), u64> {
        wait_for_tracker_drain(&self.drain_tracker, timeout).await
    }
}

#[cfg(test)]
#[path = "shutdown_drain_race_tests.rs"]
mod drain_race_tests;
