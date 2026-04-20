//! Opt-in background health probe for resource runtimes.
//!
//! [`WatchdogHandle`] runs a periodic health check loop in a background task.
//! After `failure_threshold` consecutive failures, it calls `on_health_change(false)`.
//! After `recovery_threshold` consecutive successes, it calls `on_health_change(true)`.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// Configuration for the watchdog health probe.
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    /// How often to run the health check.
    pub interval: Duration,
    /// Timeout for each probe attempt.
    pub probe_timeout: Duration,
    /// Consecutive failures before marking unhealthy.
    pub failure_threshold: u32,
    /// Consecutive successes to recover from unhealthy.
    pub recovery_threshold: u32,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(30),
            probe_timeout: Duration::from_secs(5),
            failure_threshold: 3,
            recovery_threshold: 1,
        }
    }
}

/// Mutable state tracked across probe iterations.
struct ProbeState {
    consecutive_failures: u32,
    consecutive_successes: u32,
    healthy: bool,
}

impl ProbeState {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            consecutive_successes: 0,
            healthy: true,
        }
    }

    /// Update state after a successful probe. Returns `Some(true)` on recovery transition.
    fn record_success(&mut self, recovery_threshold: u32) -> Option<bool> {
        self.consecutive_failures = 0;
        self.consecutive_successes = self.consecutive_successes.saturating_add(1);
        if !self.healthy && self.consecutive_successes >= recovery_threshold {
            self.healthy = true;
            return Some(true);
        }
        None
    }

    /// Update state after a failed probe. Returns `Some(false)` on failure transition.
    fn record_failure(&mut self, failure_threshold: u32) -> Option<bool> {
        self.consecutive_successes = 0;
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.healthy && self.consecutive_failures >= failure_threshold {
            self.healthy = false;
            return Some(false);
        }
        None
    }
}

/// Handle to a running watchdog probe.
///
/// Dropping the handle cancels the background task.
pub struct WatchdogHandle {
    cancel: CancellationToken,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl WatchdogHandle {
    /// Starts a watchdog that periodically calls `check_fn`.
    ///
    /// `on_health_change` is called when health transitions between
    /// healthy and unhealthy states.
    ///
    /// The watchdog respects `parent_cancel` for graceful shutdown.
    pub fn start<F, Fut>(
        config: WatchdogConfig,
        check_fn: F,
        on_health_change: impl Fn(bool) + Send + Sync + 'static,
        parent_cancel: CancellationToken,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), crate::Error>> + Send,
    {
        let cancel = parent_cancel.child_token();
        let token = cancel.clone();

        let join = tokio::spawn(async move {
            let mut state = ProbeState::new();

            loop {
                tokio::select! {
                    () = token.cancelled() => break,
                    () = tokio::time::sleep(config.interval) => {}
                }

                if token.is_cancelled() {
                    break;
                }

                let ok = tokio::time::timeout(config.probe_timeout, check_fn())
                    .await
                    .is_ok_and(|r| r.is_ok());

                let transition = if ok {
                    state.record_success(config.recovery_threshold)
                } else {
                    state.record_failure(config.failure_threshold)
                };

                if let Some(now_healthy) = transition {
                    on_health_change(now_healthy);
                }
            }
        });

        Self {
            cancel,
            join: Some(join),
        }
    }

    /// Stops the watchdog and waits for it to finish.
    pub async fn stop(mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

impl Drop for WatchdogHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    };

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn watchdog_detects_failure() {
        let call_count = Arc::new(AtomicU32::new(0));
        let unhealthy_fired = Arc::new(AtomicBool::new(false));

        let count = Arc::clone(&call_count);
        let check_fn = move || {
            let count = Arc::clone(&count);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Err(crate::Error::transient("probe failed"))
            }
        };

        let fired = Arc::clone(&unhealthy_fired);
        let on_change = move |healthy: bool| {
            if !healthy {
                fired.store(true, Ordering::SeqCst);
            }
        };

        let config = WatchdogConfig {
            interval: Duration::from_millis(100),
            probe_timeout: Duration::from_secs(1),
            failure_threshold: 3,
            recovery_threshold: 1,
        };

        let cancel = CancellationToken::new();
        let handle = WatchdogHandle::start(config, check_fn, on_change, cancel.clone());

        // Advance time enough for 3+ failures (3 intervals = 300ms + margin).
        tokio::time::sleep(Duration::from_millis(350)).await;
        tokio::task::yield_now().await;

        assert!(
            unhealthy_fired.load(Ordering::SeqCst),
            "expected on_health_change(false) after 3 failures"
        );
        assert!(
            call_count.load(Ordering::SeqCst) >= 3,
            "expected at least 3 probe calls"
        );

        handle.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_recovers() {
        let call_count = Arc::new(AtomicU32::new(0));
        let health_state = Arc::new(AtomicBool::new(true));

        let count = Arc::clone(&call_count);
        let check_fn = move || {
            let count = Arc::clone(&count);
            async move {
                let n = count.fetch_add(1, Ordering::SeqCst);
                // First 3 calls fail, then succeed.
                probe_result_for(n)
            }
        };

        fn probe_result_for(n: u32) -> Result<(), crate::Error> {
            if n < 3 {
                Err(crate::Error::transient("probe failed"))
            } else {
                Ok(())
            }
        }

        let state = Arc::clone(&health_state);
        let on_change = move |healthy: bool| {
            state.store(healthy, Ordering::SeqCst);
        };

        let config = WatchdogConfig {
            interval: Duration::from_millis(100),
            probe_timeout: Duration::from_secs(1),
            failure_threshold: 3,
            recovery_threshold: 1,
        };

        let cancel = CancellationToken::new();
        let handle = WatchdogHandle::start(config, check_fn, on_change, cancel.clone());

        // After 3 failures (~300ms), should go unhealthy.
        tokio::time::sleep(Duration::from_millis(350)).await;
        tokio::task::yield_now().await;
        assert!(
            !health_state.load(Ordering::SeqCst),
            "expected unhealthy after 3 failures"
        );

        // One more interval for a success -> recovery.
        tokio::time::sleep(Duration::from_millis(150)).await;
        tokio::task::yield_now().await;
        assert!(
            health_state.load(Ordering::SeqCst),
            "expected recovery after 1 success"
        );

        handle.stop().await;
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_respects_cancellation() {
        let call_count = Arc::new(AtomicU32::new(0));

        let count = Arc::clone(&call_count);
        let check_fn = move || {
            let count = Arc::clone(&count);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        };

        let config = WatchdogConfig {
            interval: Duration::from_millis(100),
            probe_timeout: Duration::from_secs(1),
            failure_threshold: 3,
            recovery_threshold: 1,
        };

        let cancel = CancellationToken::new();
        let handle = WatchdogHandle::start(config, check_fn, |_| {}, cancel.clone());

        // Let it run for a couple of ticks.
        tokio::time::sleep(Duration::from_millis(250)).await;
        tokio::task::yield_now().await;
        let before = call_count.load(Ordering::SeqCst);

        // Cancel and wait for stop.
        cancel.cancel();
        handle.stop().await;

        // Advance more time — no new calls should happen.
        tokio::time::sleep(Duration::from_millis(500)).await;
        tokio::task::yield_now().await;
        let after = call_count.load(Ordering::SeqCst);

        assert_eq!(
            before, after,
            "expected no more probe calls after cancellation"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn watchdog_timeout_counts_as_failure() {
        let unhealthy_fired = Arc::new(AtomicBool::new(false));

        // check_fn that hangs forever (simulates timeout).
        let check_fn = || async { std::future::pending::<Result<(), crate::Error>>().await };

        let fired = Arc::clone(&unhealthy_fired);
        let on_change = move |healthy: bool| {
            if !healthy {
                fired.store(true, Ordering::SeqCst);
            }
        };

        let config = WatchdogConfig {
            interval: Duration::from_millis(100),
            // Very short timeout so the pending future times out quickly.
            probe_timeout: Duration::from_millis(50),
            failure_threshold: 2,
            recovery_threshold: 1,
        };

        let cancel = CancellationToken::new();
        let handle = WatchdogHandle::start(config, check_fn, on_change, cancel.clone());

        // Each cycle: 100ms interval + 50ms timeout = ~150ms per iteration.
        // Need 2 failures, so ~300ms + margin.
        tokio::time::sleep(Duration::from_millis(400)).await;
        tokio::task::yield_now().await;

        assert!(
            unhealthy_fired.load(Ordering::SeqCst),
            "expected timeout to count as failure"
        );

        handle.stop().await;
    }
}
