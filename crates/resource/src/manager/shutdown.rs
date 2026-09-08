//! Graceful shutdown machinery for the [`Manager`].
//!
//! Phases:
//!
//! 1. **SIGNAL** — cancel the manager token, rejecting new acquires.
//! 2. **DRAIN** — wait for in-flight handles, honouring
//!    [`DrainTimeoutPolicy`].
//! 3. **RETIRE** — fence rows and submit retained-instance cleanup.
//! 4. **AWAIT WORKERS** — wait for release-queue workers to exit.
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
    events::ResourceEvent,
    manager::{DrainTimeoutPolicy, Manager, ShutdownConfig},
    release_queue::ReleaseQueue,
};

/// Waits until `tracker`'s counter reaches `0` or `timeout` elapses.
///
/// Shared by the manager-wide [`Manager::wait_for_drain`] (drains
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
    /// Does not prove success of earlier asynchronous releases or rescue completion.
    pub release_queue_drained: bool,
    /// Cumulative queue-lifetime count of release tasks abandoned before
    /// their futures completed. Detached rescue tasks and late Force-policy
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
    /// `graceful_shutdown` was already in progress when this call entered.
    /// CAS-guarded so exactly one caller wins the race.
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

    /// Phase 4 did not finish within `release_queue_timeout`.
    #[error("release queue workers did not finish within {timeout:?}")]
    ReleaseQueueTimeout {
        /// The budget that was exceeded.
        timeout: Duration,
    },

    /// A release worker failed to join. Other workers were aborted and joined;
    /// the error deliberately excludes third-party panic payloads.
    #[error("release queue worker failed")]
    ReleaseQueueWorkerFailed,

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
            Self::ResourceTeardownFailed { key, source } => formatter
                .debug_struct("ResourceTeardownFailed")
                .field("key", key)
                .field("source_kind", source.kind())
                .finish_non_exhaustive(),
        }
    }
}

/// Internal drain-phase error used by the private `wait_for_drain` helper.
/// Carries the outstanding-handle count at the moment the drain timer fired.
#[derive(Debug)]
pub(super) struct DrainTimeoutError {
    pub(super) outstanding: u64,
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

impl Manager {
    /// Triggers graceful shutdown with drain and cleanup.
    ///
    /// 1. **Signal** — cancels the token so new acquires are rejected.
    /// 2. **Drain** — waits up to [`ShutdownConfig::drain_timeout`] for in-flight handles to be
    ///    released.
    /// 3. **Retire** — fences rows, joins maintenance, and destroys retained entries.
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
    /// - [`ShutdownError::AlreadyShuttingDown`] if a concurrent caller already
    ///   won the CAS — shutdown is one-shot, not re-entrant.
    /// - [`ShutdownError::DrainTimeout`] if in-flight handles do not release
    ///   within [`ShutdownConfig::drain_timeout`] and
    ///   [`ShutdownConfig::on_drain_timeout`] is
    ///   [`DrainTimeoutPolicy::Abort`] (the registry is left un-cleared so a
    ///   caller can inspect what is still outstanding).
    /// - [`ShutdownError::ReleaseQueueTimeout`] if the release-queue workers
    ///   do not finish draining within
    ///   [`ShutdownConfig::release_queue_timeout`].
    /// - [`ShutdownError::ReleaseQueueWorkerFailed`] if a worker failed to join.
    /// - [`ShutdownError::ResourceTeardownFailed`] if a current row's terminal cleanup failed.
    ///
    /// With Force and outstanding handles, returns an incomplete report without
    /// closing cleanup. Late releases can finish while the manager stays alive.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe with respect to correctness: the shutdown flag and the
    /// cancellation token are set synchronously before the first await, so
    /// dropping this future still leaves new acquires permanently rejected
    /// throughout shutdown. During DRAIN, cancellation preserves the registry
    /// and keeps cleanup available until the manager drops. During terminal
    /// receipt waiting, already-submitted cleanup continues under queue ownership.
    /// During worker
    /// waiting, cancellation aborts unfinished workers; abort acknowledgement
    /// cannot be awaited by a dropped future. It is **not** idempotent-on-cancel:
    /// cancellation skips the remaining phases and report, and a retry
    /// immediately returns `AlreadyShuttingDown` — treat shutdown as
    /// one-shot and do not race it against a timeout you intend to retry.
    pub async fn graceful_shutdown(
        &self,
        config: ShutdownConfig,
    ) -> Result<ShutdownReport, ShutdownError> {
        // CAS idempotency guard: exactly one caller wins. Concurrent callers
        // that arrive after this CAS see `AlreadyShuttingDown` immediately
        // rather than re-entering the drain logic against a half-torn state.
        {
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

            // Mark every registered resource as `Draining` so operators polling
            // `health_check` during the drain window see the correct lifecycle
            // phase instead of a stale `Ready`.
            self.set_phase_all(crate::state::ResourcePhase::Draining);
            for managed in self.registry.all_managed() {
                managed.begin_close();
            }
        }

        // Phase 2: DRAIN — wait for in-flight handles to be released.
        // On timeout, respect the policy: Abort preserves "graceful"
        // (returns Err *without* clearing the registry), Force proceeds
        // but records the outstanding count in the report.
        let mut outstanding_after_drain: u64 = 0;
        match self.wait_for_drain(config.drain_timeout).await {
            Ok(()) => {},
            Err(DrainTimeoutError { outstanding }) => match config.on_drain_timeout {
                DrainTimeoutPolicy::Abort => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Abort — \
                         registry preserved, marking all resources Failed, \
                         returning DrainTimeout"
                    );
                    // Every resource transitions to `Failed` (with
                    // `HealthChanged{healthy:false}` emitted per key) rather
                    // than back to `Ready`. The cancel token fired in Phase 1
                    // already rejects new acquires; pretending the registry
                    // is `Ready` while the caller observes a `DrainTimeout`
                    // is phase corruption — callers polling `health_check`
                    // would see `Ready` but get `Error::cancelled` from
                    // `lookup`. Marking `Failed` makes the registry tell the
                    // truth.
                    let err = ShutdownError::DrainTimeout { outstanding };
                    self.set_phase_all_failed(&err);
                    // Do NOT reset `shutting_down` here. Both shutdown
                    // failure modes (`DrainTimeout`, `ReleaseQueueTimeout`
                    // below) are non-recoverable — the cancel token has
                    // fired and the registry has either been marked Failed
                    // or contains live handles we cannot safely re-drain.
                    // Resetting would only permit a doomed retry that races
                    // the cancel token with no benefit and risks tearing
                    // down state mid-observation by a concurrent caller.
                    return Err(err);
                },
                DrainTimeoutPolicy::Force => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Force — \
                         clearing registry anyway"
                    );
                    outstanding_after_drain = outstanding;
                },
            },
        }

        // Enqueue every row before the first await: cancellation cannot strand
        // a later row in an unobserved local Vec. Mutation and queue submission
        // share admission with remove/replacement and final registration commit.
        let receipts: Vec<_> = {
            let _admission = self
                .admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.registry
                .clear()
                .into_iter()
                .map(|managed| {
                    let key = managed.resource_key();
                    (key, self.retire_resource(managed))
                })
                .collect()
        };

        if outstanding_after_drain > 0 {
            tracing::warn!(
                outstanding = outstanding_after_drain,
                "resource manager: forced retirement incomplete; late cleanup remains open"
            );
            return Ok(ShutdownReport {
                outstanding_handles_after_drain: outstanding_after_drain,
                registry_cleared: true,
                release_queue_drained: false,
                dropped_release_tasks: self.release_queue.dropped_count(),
            });
        }

        // Maintenance may still publish cleanup jobs until its row receipt
        // completes. Keep the queue open until those producers have joined.
        let started = tokio::time::Instant::now();
        let terminal = tokio::time::timeout(config.release_queue_timeout, async {
            let mut first_error = None;
            for (key, receipt) in receipts {
                let result = receipt
                    .await
                    .unwrap_or_else(|_| Err(crate::Error::cancelled()));
                if let Err(source) = result {
                    tracing::warn!(resource.key = %key, error.kind = ?source.kind(),
                        "resource terminal cleanup failed");
                    first_error
                        .get_or_insert(ShutdownError::ResourceTeardownFailed { key, source });
                }
            }
            // Removed/replaced rows may still own maintenance producers.
            // Final admission is closed, so this set cannot grow afterward.
            let _ = wait_for_tracker_drain(&self.retirement_tracker, Duration::MAX).await;
            first_error
        })
        .await;

        self.release_queue.close();
        let handle = self.release_queue_handle.lock().await.take();
        if let Some(handle) = handle {
            let remaining = config
                .release_queue_timeout
                .saturating_sub(started.elapsed());
            ReleaseQueue::shutdown_bounded(handle, remaining)
                .await
                .map_err(|error| match error {
                    ShutdownError::ReleaseQueueTimeout { .. } => {
                        ShutdownError::ReleaseQueueTimeout {
                            timeout: config.release_queue_timeout,
                        }
                    },
                    other => other,
                })?;
        }
        match terminal {
            Err(_) => {
                return Err(ShutdownError::ReleaseQueueTimeout {
                    timeout: config.release_queue_timeout,
                });
            },
            Ok(Some(error)) => return Err(error),
            Ok(None) => {},
        }

        let dropped_release_tasks = self.release_queue.dropped_count();
        tracing::info!(dropped_release_tasks, "resource manager: shutdown complete");
        Ok(ShutdownReport {
            outstanding_handles_after_drain: outstanding_after_drain,
            registry_cleared: true,
            // If we reached this line Phase 4 either succeeded or had no
            // work to drain — either way the contract is "drained".
            release_queue_drained: true,
            dropped_release_tasks,
        })
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
    pub(super) async fn wait_for_drain(&self, timeout: Duration) -> Result<(), DrainTimeoutError> {
        wait_for_tracker_drain(&self.drain_tracker, timeout)
            .await
            .map_err(|outstanding| DrainTimeoutError { outstanding })
    }
}

#[cfg(test)]
mod drain_race_tests {
    use std::{sync::Arc, time::Instant};

    use super::*;
    use crate::manager::Manager;

    #[test]
    fn shutdown_debug_redacts_provider_error_but_preserves_source_chain() {
        let failure = ShutdownError::ResourceTeardownFailed {
            key: nebula_core::resource_key!("debug-safety"),
            source: crate::Error::permanent("PRIVATE_PROVIDER_PAYLOAD"),
        };
        for rendered in [
            format!("{failure:?}"),
            format!("{failure:#?}"),
            failure.to_string(),
        ] {
            assert!(!rendered.contains("PRIVATE_PROVIDER_PAYLOAD"));
            assert!(rendered.contains("debug-safety"));
        }
        assert!(
            std::error::Error::source(&failure)
                .unwrap()
                .to_string()
                .contains("PRIVATE_PROVIDER_PAYLOAD")
        );
    }

    #[tokio::test]
    async fn shutdown_keeps_cleanup_open_for_already_retired_producers() {
        let manager = Manager::new();
        let settlement = RetirementSettlement::new(Arc::clone(&manager.retirement_tracker));
        let queue = Arc::clone(&manager.release_queue);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let (cleaned_tx, cleaned_rx) = tokio::sync::oneshot::channel();
        // Models a removed row's maintenance producer: the registry snapshot
        // is empty, but producer ownership must keep admission to cleanup open.
        drop(manager.release_queue.submit_release(move || {
            Box::pin(async move {
                let _settlement = settlement;
                started_tx.send(()).expect("start receiver");
                resume_rx.await.expect("resume sender");
                queue.submit(move || {
                    Box::pin(async move {
                        cleaned_tx.send(()).expect("cleanup receiver");
                    })
                });
                Ok(())
            })
        }));
        started_rx.await.expect("producer started");
        let shutdown = manager.graceful_shutdown(ShutdownConfig::default());
        tokio::pin!(shutdown);
        assert!(futures::poll!(shutdown.as_mut()).is_pending());
        resume_tx.send(()).expect("producer still owned");
        let report = shutdown.await.expect("producer and workers drain");
        cleaned_rx.await.expect("late cleanup was accepted");
        assert!(report.release_queue_drained);
        assert_eq!(report.dropped_release_tasks, 0);
    }

    #[tokio::test]
    async fn cancelling_manager_worker_wait_releases_task_ownership() {
        let manager = Manager::new();
        let ownership = Arc::new(());
        let lease = Arc::clone(&ownership);
        manager.release_queue.submit(move || {
            Box::pin(async move {
                std::future::pending::<()>().await;
                drop(lease);
            })
        });
        {
            let shutdown = manager.graceful_shutdown(ShutdownConfig::default());
            tokio::pin!(shutdown);
            assert!(futures::poll!(shutdown.as_mut()).is_pending());
        }
        tokio::task::yield_now().await;
        assert_eq!(Arc::strong_count(&ownership), 1);
        assert_eq!(manager.release_queue.dropped_count(), 1);
        assert!(matches!(
            manager.graceful_shutdown(ShutdownConfig::default()).await,
            Err(ShutdownError::AlreadyShuttingDown)
        ));
    }

    #[tokio::test]
    async fn manager_drop_closes_queue_even_with_external_queue_reference() {
        let manager = Manager::new();
        let queue = Arc::clone(&manager.release_queue);
        let handle = manager.release_queue_handle.lock().await.take().unwrap();
        drop(manager);
        ReleaseQueue::shutdown(handle).await;
        queue.submit(|| panic!("manager drop closes queue"));
        assert_eq!(queue.dropped_count(), 1);
    }

    /// Regression for the drain-race bug: previously `wait_for_drain`
    /// did `tracker.1.notified().await` without pre-registering the
    /// `Notified` future, so a handle dropping (and firing
    /// `notify_waiters()`) in the window between the outer
    /// `active == 0` check and the first `notified().await` poll would
    /// leak the wakeup. Stall persisted until the full `drain_timeout`
    /// elapsed.
    ///
    /// The fix pre-enables the `Notified` future and re-checks the
    /// counter *after* registration, so a drop that completes the drain
    /// mid-race is observed on the re-check and returns immediately.
    ///
    /// This test exercises the normal "handle drops while we're waiting"
    /// path and asserts we return far sooner than the timeout.
    #[tokio::test]
    async fn wait_for_drain_returns_promptly_when_handle_drops() {
        let mgr = Manager::new();
        // Simulate one active handle.
        mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

        let tracker = Arc::clone(&mgr.drain_tracker);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
                tracker.1.notify_waiters();
            }
        });

        let start = Instant::now();
        mgr.wait_for_drain(Duration::from_secs(30))
            .await
            .expect("handle drop must drain under the timeout");
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "wait_for_drain should return within 1s when a handle drops, took {elapsed:?}"
        );
        assert_eq!(mgr.drain_tracker.0.load(AtomicOrdering::Acquire), 0);
    }

    /// Regression: if the counter reaches 0 *before* `wait_for_drain`
    /// gets to pre-register the `Notified`, the post-enable re-check
    /// must catch it and return immediately rather than stalling.
    ///
    /// We simulate the race by setting `active = 1` (so the outer
    /// early-return doesn't fire), then immediately decrementing to 0
    /// before `wait_for_drain` is polled.
    #[tokio::test]
    async fn wait_for_drain_catches_drop_via_recheck() {
        let mgr = Manager::new();
        mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

        // Decrement + notify synchronously — the counter is 0 before
        // `wait_for_drain` is even called, but we want to prove that
        // even if the outer check observed `active == 1` and then
        // the counter hit 0 *between* that check and the inner enable,
        // the inner re-check would catch it.
        //
        // Simulated here by priming the state and then calling
        // wait_for_drain directly; the inner loop's re-check should
        // fire on the very first iteration because the counter is
        // already 0. The outer check is bypassed by the fetch_add
        // above leaving active == 1 until... wait, we need to
        // decrement BETWEEN the outer check and the inner enable.
        //
        // Easiest approximation: skip the outer early-return by
        // keeping active = 1 through the outer check, then decrement
        // via a spawned task that runs before wait_for_drain gets
        // scheduler time.
        let tracker = Arc::clone(&mgr.drain_tracker);
        tokio::task::yield_now().await;
        let handle = tokio::spawn(async move {
            // Yield so that wait_for_drain's outer load sees active = 1,
            // then decrement before the inner poll happens.
            tokio::task::yield_now().await;
            if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
                tracker.1.notify_waiters();
            }
        });

        let start = Instant::now();
        mgr.wait_for_drain(Duration::from_secs(30))
            .await
            .expect("recheck path must drain under the timeout");
        let elapsed = start.elapsed();
        handle.await.unwrap();

        assert!(
            elapsed < Duration::from_secs(1),
            "wait_for_drain must return promptly even under race, took {elapsed:?}"
        );
    }

    /// #302: Abort policy must return a typed `DrainTimeout` error and
    /// leave the registry untouched. Before the policy split
    /// `graceful_shutdown` would log a warning and proceed to
    /// `registry.clear()` anyway, turning a cooperative shutdown into a
    /// logical use-after-free.
    #[tokio::test]
    async fn graceful_shutdown_abort_policy_returns_drain_timeout_error() {
        let mgr = Manager::new();
        // Simulate an outstanding handle.
        mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

        let cfg = ShutdownConfig::default()
            .with_drain_timeout(Duration::from_millis(50))
            .with_drain_timeout_policy(DrainTimeoutPolicy::Abort);

        let err = mgr
            .graceful_shutdown(cfg)
            .await
            .expect_err("Abort policy must surface drain timeout");
        match err {
            ShutdownError::DrainTimeout { outstanding } => {
                assert_eq!(outstanding, 1, "outstanding count mismatch");
            },
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    /// #302: Force policy must clear the registry and report the
    /// outstanding-handle count in `ShutdownReport` so operators can see
    /// exactly how much in-flight work was abandoned.
    #[tokio::test]
    async fn graceful_shutdown_force_policy_clears_registry_with_outstanding_count() {
        let mgr = Manager::new();
        mgr.drain_tracker.0.fetch_add(2, AtomicOrdering::Release);

        let cfg = ShutdownConfig::default()
            .with_drain_timeout(Duration::from_millis(50))
            .with_drain_timeout_policy(DrainTimeoutPolicy::Force);

        let report = mgr
            .graceful_shutdown(cfg)
            .await
            .expect("Force policy must succeed");
        assert!(report.registry_cleared);
        assert_eq!(report.outstanding_handles_after_drain, 2);
    }
}
