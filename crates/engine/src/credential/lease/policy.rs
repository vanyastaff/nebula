//! Renewal policy for [`LeaseLifecycle`](super::LeaseLifecycle).
//!
//! Defaults follow the Vault Agent recommendation: renew at 70% of the
//! issued TTL, then bounded exponential backoff `[1s, 2s, 4s, 8s, 16s]`
//! with a five-attempt budget before the lease is dropped.

use std::time::Duration;

/// Policy controlling when the lifecycle renews a tracked lease and how
/// it backs off on transient failure.
///
/// The defaults match the standard HashiCorp Vault Agent guidance — 70%
/// renewal point, exponential backoff, finite retry budget. Production
/// composition is expected to keep the defaults; the fields are public
/// for tests that need a tight clock.
#[derive(Debug, Clone)]
pub struct RenewalPolicy {
    /// Fraction of the issued TTL at which renewal fires. Must be in
    /// `(0.0, 1.0]`. Default `0.7`.
    pub ratio: f32,
    /// Backoff schedule on `Unavailable` / `Backend` errors. The lease
    /// is dropped after `backoff.len()` consecutive failures even before
    /// `max_retries` is consulted — both bounds protect against runaway
    /// renewal storms.
    pub backoff: Vec<Duration>,
    /// Maximum consecutive renewal failures before the lease is dropped.
    /// Set as a guard alongside `backoff.len()` so callers can tune
    /// retry budget independently from the wait schedule.
    pub max_retries: u32,
}

impl Default for RenewalPolicy {
    fn default() -> Self {
        Self {
            ratio: 0.7,
            backoff: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
            ],
            max_retries: 5,
        }
    }
}

impl RenewalPolicy {
    /// Compute the next renewal delay relative to lease issue time given
    /// the provider-reported TTL. Honours the policy ratio.
    #[must_use]
    pub fn renew_after(&self, ttl: Duration) -> Duration {
        let clamped_ratio = self.ratio.clamp(f32::MIN_POSITIVE, 1.0);
        let secs_f = ttl.as_secs_f64() * f64::from(clamped_ratio);
        // `Duration::from_secs_f64` panics on `NaN` / `Inf`; convert
        // defensively through a finite check.
        if secs_f.is_finite() && secs_f >= 0.0 {
            Duration::from_secs_f64(secs_f)
        } else {
            Duration::ZERO
        }
    }

    /// Backoff duration for the `attempt`-th consecutive failure
    /// (0-indexed). Returns `None` when the retry budget is exhausted.
    #[must_use]
    pub fn backoff_for(&self, attempt: u32) -> Option<Duration> {
        if attempt >= self.max_retries {
            return None;
        }
        let idx = usize::try_from(attempt).unwrap_or(usize::MAX);
        self.backoff.get(idx).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_renew_after_picks_seventy_percent() {
        let p = RenewalPolicy::default();
        let after = p.renew_after(Duration::from_secs(100));
        // f32 ratio carries ~1ms of imprecision at the 100s scale; allow
        // a small tolerance rather than demanding exact equality.
        let target = Duration::from_secs(70);
        let diff = after.abs_diff(target);
        assert!(
            diff < Duration::from_millis(10),
            "after={after:?}, target={target:?}"
        );
    }

    #[test]
    fn renew_after_ratio_clamped_to_unit_interval() {
        let p = RenewalPolicy {
            ratio: 1.5,
            ..RenewalPolicy::default()
        };
        // Clamps to 1.0 → full TTL.
        let after = p.renew_after(Duration::from_secs(10));
        assert_eq!(after, Duration::from_secs(10));
    }

    #[test]
    fn renew_after_zero_ttl_yields_zero() {
        let p = RenewalPolicy::default();
        assert_eq!(p.renew_after(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn backoff_schedule_exhausts_after_budget() {
        let p = RenewalPolicy::default();
        assert_eq!(p.backoff_for(0), Some(Duration::from_secs(1)));
        assert_eq!(p.backoff_for(4), Some(Duration::from_secs(16)));
        assert_eq!(p.backoff_for(5), None);
        assert_eq!(p.backoff_for(100), None);
    }
}
