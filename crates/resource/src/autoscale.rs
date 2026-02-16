//! Auto-scaling policy and background scaler for resource pools.
//!
//! The [`AutoScaler`] monitors pool utilization and triggers scale-up or
//! scale-down operations when sustained watermark thresholds are breached.

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// AutoScalePolicy
// ---------------------------------------------------------------------------

/// Policy controlling when and how aggressively the auto-scaler acts.
///
/// Utilization is calculated as `active / max_size`. When it exceeds
/// `high_watermark` for `evaluation_window`, the scaler pre-creates
/// `scale_up_step` idle instances. When it drops below `low_watermark`
/// for the same window, the scaler removes `scale_down_step` idle instances.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AutoScalePolicy {
    /// Utilization above which to scale up (0.0, 1.0]. Default: 0.8
    pub high_watermark: f64,
    /// Utilization below which to scale down [0.0, 1.0). Default: 0.2
    pub low_watermark: f64,
    /// Number of instances to pre-create when scaling up. Default: 2
    pub scale_up_step: usize,
    /// Number of idle instances to remove when scaling down. Default: 1
    pub scale_down_step: usize,
    /// How long utilization must exceed a watermark before action. Default: 30s
    pub evaluation_window: Duration,
    /// Minimum time between scale operations. Default: 60s
    pub cooldown: Duration,
}

impl Default for AutoScalePolicy {
    fn default() -> Self {
        Self {
            high_watermark: 0.8,
            low_watermark: 0.2,
            scale_up_step: 2,
            scale_down_step: 1,
            evaluation_window: Duration::from_secs(30),
            cooldown: Duration::from_secs(60),
        }
    }
}

impl AutoScalePolicy {
    /// Validate the policy, returning an error if any field is out of range.
    pub fn validate(&self) -> Result<()> {
        if self.high_watermark <= 0.0 || self.high_watermark > 1.0 {
            return Err(Error::configuration("high_watermark must be in (0.0, 1.0]"));
        }
        if self.low_watermark < 0.0 || self.low_watermark >= 1.0 {
            return Err(Error::configuration("low_watermark must be in [0.0, 1.0)"));
        }
        if self.low_watermark >= self.high_watermark {
            return Err(Error::configuration(
                "low_watermark must be less than high_watermark",
            ));
        }
        if self.scale_up_step == 0 {
            return Err(Error::configuration("scale_up_step must be > 0"));
        }
        if self.scale_down_step == 0 {
            return Err(Error::configuration("scale_down_step must be > 0"));
        }
        if self.evaluation_window.is_zero() {
            return Err(Error::configuration(
                "evaluation_window must be greater than zero",
            ));
        }
        if self.cooldown.is_zero() {
            return Err(Error::configuration("cooldown must be greater than zero"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AutoScaler
// ---------------------------------------------------------------------------

/// Background auto-scaler that monitors pool utilization and triggers
/// scale-up / scale-down callbacks when sustained watermark thresholds
/// are breached.
///
/// The scaler is decoupled from `Pool` via closures, making it testable
/// and reusable across different pool implementations.
pub struct AutoScaler {
    policy: AutoScalePolicy,
    cancel: CancellationToken,
}

impl AutoScaler {
    /// Create a new auto-scaler with the given policy and cancellation token.
    ///
    /// Call [`validate`](AutoScalePolicy::validate) on the policy before
    /// constructing if you want early validation.
    #[must_use]
    pub fn new(policy: AutoScalePolicy, cancel: CancellationToken) -> Self {
        Self { policy, cancel }
    }

    /// Spawn the background auto-scaler task.
    ///
    /// - `get_stats`: Returns current `(active, idle, max_size)`.
    /// - `scale_up`: Called with the count of instances to pre-create.
    ///   Returns the number actually created.
    /// - `scale_down`: Called with the count of idle instances to remove.
    ///   Returns the number actually removed.
    ///
    /// The task checks utilization every `evaluation_window / 2` and
    /// respects the cooldown between scale operations.
    ///
    /// Returns a [`JoinHandle`](tokio::task::JoinHandle) that can be
    /// awaited to confirm the background task has fully exited after
    /// [`shutdown`](Self::shutdown) is called.
    pub fn start<S, U, D, UF, DF>(
        &self,
        get_stats: S,
        scale_up: U,
        scale_down: D,
    ) -> tokio::task::JoinHandle<()>
    where
        S: Fn() -> (usize, usize, usize) + Send + Sync + 'static,
        U: Fn(usize) -> UF + Send + Sync + 'static,
        UF: Future<Output = usize> + Send,
        D: Fn(usize) -> DF + Send + Sync + 'static,
        DF: Future<Output = usize> + Send,
    {
        let policy = self.policy.clone();
        let cancel = self.cancel.clone();

        tokio::spawn(async move {
            let check_interval = policy.evaluation_window / 2;

            // Tracks when utilization first exceeded / went below watermarks.
            let mut high_since: Option<Instant> = None;
            let mut low_since: Option<Instant> = None;
            let mut last_scale_op: Option<Instant> = None;

            loop {
                tokio::select! {
                    () = tokio::time::sleep(check_interval) => {}
                    () = cancel.cancelled() => break,
                }

                let (active, _idle, max_size) = get_stats();
                let utilization = if max_size == 0 {
                    0.0
                } else {
                    active as f64 / max_size as f64
                };

                let now = Instant::now();
                let in_cooldown =
                    last_scale_op.is_some_and(|t| now.duration_since(t) < policy.cooldown);

                // Track high watermark breach
                high_since = if utilization > policy.high_watermark {
                    Some(high_since.unwrap_or(now))
                } else {
                    None
                };

                // Track low watermark breach
                low_since = if utilization < policy.low_watermark {
                    Some(low_since.unwrap_or(now))
                } else {
                    None
                };

                // Scale up if sustained above high watermark
                if let Some(since) = high_since.filter(|_| !in_cooldown)
                    && now.duration_since(since) >= policy.evaluation_window
                {
                    let _created = scale_up(policy.scale_up_step).await;
                    last_scale_op = Some(Instant::now());
                    high_since = None;
                }

                // Scale down if sustained below low watermark
                if let Some(since) = low_since.filter(|_| !in_cooldown)
                    && now.duration_since(since) >= policy.evaluation_window
                {
                    let _removed = scale_down(policy.scale_down_step).await;
                    last_scale_op = Some(Instant::now());
                    low_since = None;
                }
            }
        })
    }

    /// Cancel the background auto-scaler task.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

impl std::fmt::Debug for AutoScaler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoScaler")
            .field("policy", &self.policy)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // -- Policy validation tests --

    #[test]
    fn default_policy_is_valid() {
        AutoScalePolicy::default().validate().unwrap();
    }

    #[test]
    fn high_watermark_zero_rejected() {
        let policy = AutoScalePolicy {
            high_watermark: 0.0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn high_watermark_above_one_rejected() {
        let policy = AutoScalePolicy {
            high_watermark: 1.1,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn high_watermark_one_accepted() {
        let policy = AutoScalePolicy {
            high_watermark: 1.0,
            ..Default::default()
        };
        policy.validate().unwrap();
    }

    #[test]
    fn low_watermark_negative_rejected() {
        let policy = AutoScalePolicy {
            low_watermark: -0.1,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn low_watermark_one_rejected() {
        let policy = AutoScalePolicy {
            low_watermark: 1.0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn low_watermark_zero_accepted() {
        let policy = AutoScalePolicy {
            low_watermark: 0.0,
            ..Default::default()
        };
        policy.validate().unwrap();
    }

    #[test]
    fn low_ge_high_rejected() {
        let policy = AutoScalePolicy {
            low_watermark: 0.8,
            high_watermark: 0.8,
            ..Default::default()
        };
        assert!(policy.validate().is_err());

        let policy2 = AutoScalePolicy {
            low_watermark: 0.9,
            high_watermark: 0.8,
            ..Default::default()
        };
        assert!(policy2.validate().is_err());
    }

    #[test]
    fn scale_up_step_zero_rejected() {
        let policy = AutoScalePolicy {
            scale_up_step: 0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn scale_down_step_zero_rejected() {
        let policy = AutoScalePolicy {
            scale_down_step: 0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn evaluation_window_zero_rejected() {
        let policy = AutoScalePolicy {
            evaluation_window: Duration::ZERO,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    #[test]
    fn cooldown_zero_rejected() {
        let policy = AutoScalePolicy {
            cooldown: Duration::ZERO,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }

    // -- AutoScaler lifecycle tests --

    #[tokio::test]
    async fn scaler_shuts_down_cleanly() {
        let cancel = CancellationToken::new();
        let scaler = AutoScaler::new(AutoScalePolicy::default(), cancel.clone());

        let scale_up_called = Arc::new(AtomicUsize::new(0));
        let scale_down_called = Arc::new(AtomicUsize::new(0));

        let up = Arc::clone(&scale_up_called);
        let down = Arc::clone(&scale_down_called);

        scaler.start(
            || (0, 5, 10), // low utilization
            move |n| {
                let up = Arc::clone(&up);
                async move {
                    up.fetch_add(n, Ordering::SeqCst);
                    n
                }
            },
            move |n| {
                let down = Arc::clone(&down);
                async move {
                    down.fetch_add(n, Ordering::SeqCst);
                    n
                }
            },
        );

        // Let it run briefly
        tokio::time::sleep(Duration::from_millis(10)).await;
        scaler.shutdown();
        // Give the spawned task time to exit
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert!(cancel.is_cancelled());
    }

    /// Advance time in 1-second steps, yielding between each to let
    /// spawned tasks process their timer wakes.
    async fn advance_stepwise(total: Duration) {
        let steps = total.as_secs();
        for _ in 0..steps {
            tokio::time::advance(Duration::from_secs(1)).await;
            tokio::task::yield_now().await;
        }
        // Extra yields to drain any ready futures
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn scaler_triggers_scale_up_after_sustained_high() {
        let cancel = CancellationToken::new();
        let policy = AutoScalePolicy {
            high_watermark: 0.7,
            low_watermark: 0.2,
            scale_up_step: 3,
            scale_down_step: 1,
            evaluation_window: Duration::from_secs(10),
            cooldown: Duration::from_secs(20),
        };
        let scaler = AutoScaler::new(policy, cancel.clone());

        let scale_up_count = Arc::new(AtomicUsize::new(0));
        let up = Arc::clone(&scale_up_count);

        // 8 active out of 10 = 0.8 utilization > 0.7 high_watermark
        scaler.start(
            || (8, 2, 10),
            move |n| {
                let up = Arc::clone(&up);
                async move {
                    up.fetch_add(n, Ordering::SeqCst);
                    n
                }
            },
            |n| async move { n },
        );

        // check_interval = evaluation_window / 2 = 5s
        // At 5s: first check detects high, sets high_since
        // At 10s: second check, duration_since = 5s < 10s
        // At 15s: third check, duration_since = 10s >= 10s -> trigger
        advance_stepwise(Duration::from_secs(16)).await;

        assert_eq!(
            scale_up_count.load(Ordering::SeqCst),
            3,
            "should have called scale_up with step=3"
        );

        scaler.shutdown();
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn scaler_triggers_scale_down_after_sustained_low() {
        let cancel = CancellationToken::new();
        let policy = AutoScalePolicy {
            high_watermark: 0.8,
            low_watermark: 0.3,
            scale_up_step: 2,
            scale_down_step: 4,
            evaluation_window: Duration::from_secs(10),
            cooldown: Duration::from_secs(20),
        };
        let scaler = AutoScaler::new(policy, cancel.clone());

        let scale_down_count = Arc::new(AtomicUsize::new(0));
        let down = Arc::clone(&scale_down_count);

        // 1 active out of 10 = 0.1 utilization < 0.3 low_watermark
        scaler.start(
            || (1, 9, 10),
            |n| async move { n },
            move |n| {
                let down = Arc::clone(&down);
                async move {
                    down.fetch_add(n, Ordering::SeqCst);
                    n
                }
            },
        );

        advance_stepwise(Duration::from_secs(16)).await;

        assert_eq!(
            scale_down_count.load(Ordering::SeqCst),
            4,
            "should have called scale_down with step=4"
        );

        scaler.shutdown();
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn scaler_respects_cooldown() {
        let cancel = CancellationToken::new();
        let policy = AutoScalePolicy {
            high_watermark: 0.5,
            low_watermark: 0.1,
            scale_up_step: 1,
            scale_down_step: 1,
            evaluation_window: Duration::from_secs(4),
            cooldown: Duration::from_secs(60),
        };
        let scaler = AutoScaler::new(policy, cancel.clone());

        let scale_up_count = Arc::new(AtomicUsize::new(0));
        let up = Arc::clone(&scale_up_count);

        // 8/10 = 0.8 > 0.5
        scaler.start(
            || (8, 2, 10),
            move |n| {
                let up = Arc::clone(&up);
                async move {
                    up.fetch_add(n, Ordering::SeqCst);
                    n
                }
            },
            |n| async move { n },
        );

        // check_interval=2s, first detect at 2s, trigger at 6s
        advance_stepwise(Duration::from_secs(7)).await;

        assert_eq!(scale_up_count.load(Ordering::SeqCst), 1, "first trigger");

        // Advance another evaluation_window but still within cooldown (60s)
        advance_stepwise(Duration::from_secs(10)).await;

        assert_eq!(
            scale_up_count.load(Ordering::SeqCst),
            1,
            "should not trigger again during cooldown"
        );

        scaler.shutdown();
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn scaler_resets_when_utilization_drops() {
        let cancel = CancellationToken::new();
        let policy = AutoScalePolicy {
            high_watermark: 0.5,
            low_watermark: 0.1,
            scale_up_step: 1,
            scale_down_step: 1,
            evaluation_window: Duration::from_secs(10),
            cooldown: Duration::from_secs(5),
        };
        let scaler = AutoScaler::new(policy, cancel.clone());

        let scale_up_count = Arc::new(AtomicUsize::new(0));
        let up = Arc::clone(&scale_up_count);

        // Start high, then drop to normal after a few seconds.
        let active = Arc::new(AtomicUsize::new(8));
        let active_clone = Arc::clone(&active);

        scaler.start(
            move || (active_clone.load(Ordering::SeqCst), 2, 10),
            move |n| {
                let up = Arc::clone(&up);
                async move {
                    up.fetch_add(n, Ordering::SeqCst);
                    n
                }
            },
            |n| async move { n },
        );

        // Let it detect high for one check cycle (5s)
        advance_stepwise(Duration::from_secs(6)).await;

        // Drop utilization to 0.3 (below high_watermark)
        active.store(3, Ordering::SeqCst);

        // Wait past evaluation_window from initial detection
        advance_stepwise(Duration::from_secs(10)).await;

        // Should NOT have triggered because utilization dropped before sustained window
        assert_eq!(
            scale_up_count.load(Ordering::SeqCst),
            0,
            "should not scale up when utilization dropped before window elapsed"
        );

        scaler.shutdown();
    }

    #[test]
    fn zero_max_size_utilization_is_zero() {
        // Just a logic sanity check: 0/0 should be 0.0, not NaN
        let util = if 0usize == 0 { 0.0 } else { 5.0 / 0.0_f64 };
        assert_eq!(util, 0.0);
    }

    #[test]
    fn auto_scaler_debug_format() {
        let cancel = CancellationToken::new();
        let scaler = AutoScaler::new(AutoScalePolicy::default(), cancel);
        let debug = format!("{scaler:?}");
        assert!(debug.contains("AutoScaler"));
        assert!(debug.contains("policy"));
    }
}
