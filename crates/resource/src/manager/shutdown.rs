//! Graceful shutdown machinery for the [`Manager`](super::Manager).
//!
//! Phases:
//!
//! 1. **SIGNAL** — cancel the shared token (rejects new acquires; signals workers to drain).
//! 2. **DRAIN** — wait for in-flight handles, honouring
//!    [`DrainTimeoutPolicy`](super::DrainTimeoutPolicy).
//! 3. **CLEAR** — drop registry entries.
//! 4. **AWAIT WORKERS** — wait for release-queue workers to exit.
//!
//! Errors are typed [`ShutdownError`] variants; the previous behaviour of
//! silently force-clearing the registry on drain timeout is now opt-in
//! through [`DrainTimeoutPolicy::Force`](super::DrainTimeoutPolicy::Force).

use std::{sync::atomic::Ordering as AtomicOrdering, time::Duration};

use crate::{
    events::ResourceEvent,
    manager::{DrainTimeoutPolicy, Manager, ShutdownConfig},
    release_queue::ReleaseQueue,
};

/// Structured result of a successful (or forced-through) graceful shutdown.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// How many `ResourceGuard`s were still outstanding when the drain
    /// phase finished. Zero on the happy path. Nonzero only when the
    /// caller explicitly opted into [`DrainTimeoutPolicy::Force`].
    pub outstanding_handles_after_drain: u64,
    /// Whether Phase 3 (`registry.clear`) actually ran.
    pub registry_cleared: bool,
    /// Whether Phase 4 (release-queue drain) completed within
    /// `release_queue_timeout`.
    pub release_queue_drained: bool,
}

/// Errors returned by [`Manager::graceful_shutdown`].
///
/// Each variant corresponds to a failure mode that was previously silently
/// absorbed by the old infallible signature. A timeout during drain, for
/// example, used to be a `tracing::warn!` and a forced `registry.clear()`;
/// it is now a typed error that the caller must handle.
#[derive(Debug, thiserror::Error)]
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
    /// post-abort reality (R-023).
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
}

/// Internal drain-phase error used by the private `wait_for_drain` helper.
/// Carries the outstanding-handle count at the moment the drain timer fired.
#[derive(Debug)]
pub(super) struct DrainTimeoutError {
    pub(super) outstanding: u64,
}

impl Manager {
    /// Triggers graceful shutdown with drain and cleanup.
    ///
    /// 1. **Signal** — cancels the token so new acquires are rejected.
    /// 2. **Drain** — waits up to [`ShutdownConfig::drain_timeout`] for in-flight handles to be
    ///    released.
    /// 3. **Clear** — drops all managed resources, releasing their `Arc<ReleaseQueue>` references
    ///    so workers can drain and exit.
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
    pub async fn graceful_shutdown(
        &self,
        config: ShutdownConfig,
    ) -> Result<ShutdownReport, ShutdownError> {
        // CAS idempotency guard: exactly one caller wins. Concurrent callers
        // that arrive after this CAS see `AlreadyShuttingDown` immediately
        // rather than re-entering the drain logic against a half-torn state.
        if self
            .shutting_down
            .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_err()
        {
            return Err(ShutdownError::AlreadyShuttingDown);
        }

        tracing::info!("resource manager: starting graceful shutdown");

        // Phase 1: SIGNAL — cancel the shared token. This does two things:
        //   a) Rejects new acquire calls (checked in `lookup`).
        //   b) Tells release queue workers to drain remaining tasks and exit
        //      (they share this token via `ReleaseQueue::with_cancel`).
        self.cancel.cancel();

        // #387: mark every registered resource as `Draining` so operators
        // polling `health_check` during the drain window see the correct
        // lifecycle phase instead of a stale `Ready`.
        self.set_phase_all(crate::state::ResourcePhase::Draining);

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
                    // R-023 / 🔴-4: every resource transitions to `Failed`
                    // (with `HealthChanged{healthy:false}` emitted per key)
                    // rather than back to `Ready`. The cancel token fired
                    // in Phase 1 already rejects new acquires; pretending
                    // the registry is `Ready` while the caller observes a
                    // `DrainTimeout` is phase corruption — callers polling
                    // `health_check` would see `Ready` but get
                    // `Error::cancelled` from `lookup`. Marking `Failed`
                    // makes the registry tell the truth.
                    let err = ShutdownError::DrainTimeout { outstanding };
                    self.set_phase_all_failed(&err);
                    // CodeRabbit 🟠 #4: do NOT reset `shutting_down` here.
                    // Both shutdown failure modes (`DrainTimeout`,
                    // `ReleaseQueueTimeout` below) are non-recoverable —
                    // the cancel token has fired and the registry has
                    // either been marked Failed or contains live handles
                    // we cannot safely re-drain. Resetting would only
                    // permit a doomed retry that races the cancel token
                    // with no benefit and risks tearing down state mid-
                    // observation by a concurrent caller.
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

        // #387: drain has completed (or been force-released). Mark every
        // resource as `ShuttingDown` so a health snapshot captured in the
        // narrow window between here and `registry.clear()` reflects the
        // real lifecycle state.
        self.set_phase_all(crate::state::ResourcePhase::ShuttingDown);

        // Phase 3: CLEAR — drop all ManagedResources so their
        // Arc<ReleaseQueue> refs are released. Also clear the
        // `credential_resources` reverse-index so its dispatchers (which
        // each hold an `Arc<ManagedResource<R>>` via `TypedDispatcher`)
        // do not pin the now-removed resources alive across any surviving
        // `Arc<Manager>` clones (CodeRabbit 🔴 #2).
        self.registry.clear();
        self.credential_resources.clear();

        // Phase 4: AWAIT WORKERS — workers are already draining (from
        // Phase 1 cancel signal). Await with a bounded timeout; failure
        // to finish in time is a typed error, not a swallowed warning.
        if let Some(handle) = self.release_queue_handle.lock().await.take() {
            let shutdown_fut = ReleaseQueue::shutdown(handle);
            if tokio::time::timeout(config.release_queue_timeout, shutdown_fut)
                .await
                .is_err()
            {
                tracing::warn!(
                    timeout = ?config.release_queue_timeout,
                    "resource manager: release queue workers did not \
                     finish within release_queue_timeout"
                );
                return Err(ShutdownError::ReleaseQueueTimeout {
                    timeout: config.release_queue_timeout,
                });
            }
        }

        tracing::info!("resource manager: shutdown complete");
        Ok(ShutdownReport {
            outstanding_handles_after_drain: outstanding_after_drain,
            registry_cleared: true,
            // If we reached this line Phase 4 either succeeded or had no
            // work to drain — either way the contract is "drained".
            release_queue_drained: true,
        })
    }

    /// Drives every registered resource to the given lifecycle phase.
    ///
    /// Type-erased bulk update used during graceful shutdown so that
    /// `health_check` returns the correct phase while the drain/cleanup
    /// is in flight (#387).
    pub(super) fn set_phase_all(&self, phase: crate::state::ResourcePhase) {
        for managed in self.registry.all_managed() {
            managed.set_phase_erased(phase);
        }
    }

    /// Marks every registered resource as `Failed` with the supplied
    /// shutdown error and emits a per-resource
    /// [`ResourceEvent::HealthChanged`] with `healthy: false`.
    ///
    /// Used by the [`DrainTimeoutPolicy::Abort`] branch (R-023) so the
    /// registry's recorded phase agrees with the `Err(DrainTimeout)`
    /// observed by the caller. Without this, `health_check` would report
    /// `Ready` while `lookup` rejected acquires via the cancel token —
    /// the exact "phase corruption" called out in Phase 1 finding 🔴-4.
    ///
    /// Broadcast send errors (no live subscribers) are intentionally
    /// ignored, matching the rest of the manager's event-emission policy.
    pub(super) fn set_phase_all_failed(&self, error: &ShutdownError) {
        let reason = error.to_string();
        for managed in self.registry.all_managed() {
            managed.set_failed_erased(&reason);
            let _ = self.event_tx.send(ResourceEvent::HealthChanged {
                key: managed.resource_key(),
                healthy: false,
            });
        }
    }

    /// Waits until all active `ResourceGuard`s are dropped or timeout expires.
    ///
    /// The loop uses a `register-then-check` ordering to avoid the classic
    /// `Notify::notify_waiters` lost-wakeup:
    ///
    /// 1. Construct + pin + `enable()` a fresh `Notified` future. Calling `enable()` registers this
    ///    waiter on the `Notify` queue without requiring a `.await`, so any subsequent
    ///    `notify_waiters()` (fired when a handle's `Drop` decrements the counter from 1 → 0) will
    ///    reach us.
    /// 2. Re-check the counter. If it already hit 0 between the outer initial check and our
    ///    registration, return now — the wakeup we would otherwise wait for has already been
    ///    consumed.
    /// 3. Only then await the `Notified` future.
    ///
    /// Without this ordering, a burst of handle drops that completes the
    /// drain *before* the first `notified().await` poll would leak the
    /// notification entirely, stalling `graceful_shutdown` for the full
    /// `drain_timeout` (default 30 s) and risking `SIGKILL` escalation
    /// under a tight orchestrator shutdown window.
    pub(super) async fn wait_for_drain(&self, timeout: Duration) -> Result<(), DrainTimeoutError> {
        let active = self.drain_tracker.0.load(AtomicOrdering::Acquire);
        if active == 0 {
            return Ok(());
        }

        tracing::debug!(active_handles = active, "waiting for handles to drain");
        let tracker = &self.drain_tracker;
        let drained = tokio::time::timeout(timeout, async {
            loop {
                // Pre-register this waiter BEFORE re-checking the counter.
                let notified = tracker.1.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                // Re-check after registration. If the last handle dropped
                // while we were between the outer check and `enable()`,
                // the counter is now 0 and we would otherwise wait on a
                // notification that has already fired.
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
            return Err(DrainTimeoutError { outstanding });
        }
        Ok(())
    }
}

#[cfg(test)]
mod drain_race_tests {
    use std::{sync::Arc, time::Instant};

    use super::*;
    use crate::manager::Manager;

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
