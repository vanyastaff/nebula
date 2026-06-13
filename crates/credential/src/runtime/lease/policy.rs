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

/// Framework-imposed upper bound on how long a leased secret may go without
/// re-validation — the lease analogue of the static credential's mandatory
/// re-validation floor (the resolver's `DEFAULT_REVALIDATION_FLOOR`).
///
/// Renewal normally fires at [`RenewalPolicy::renew_after`] (a fraction of the
/// provider-reported TTL). This ceiling caps that interval so a provider that
/// reports an enormous — or `Duration::MAX` — TTL is still renewed no later
/// than the ceiling: the "leased secret that silently never re-validates"
/// failure mode is closed by construction. The ceiling **itself** is
/// constructor-validated, so it cannot be set to zero or to an
/// effectively-unbounded value (`Duration::MAX` is rejected by [`new`]).
///
/// [`new`]: Self::new
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalenessCeiling(Duration);

impl StalenessCeiling {
    /// Hard cap the ceiling may not exceed. A ceiling at (or near)
    /// `Duration::MAX` would defeat its own purpose — it must force *some*
    /// re-validation — so any bound above this, including `Duration::MAX`, is
    /// rejected by [`new`](Self::new). Seven days.
    pub const HARD_CAP: Duration = Duration::from_hours(24 * 7);

    /// Build a ceiling from `bound`, rejecting a zero or
    /// above-[`HARD_CAP`](Self::HARD_CAP) value so `Duration::MAX` is
    /// unconstructible on the lease path.
    ///
    /// # Errors
    ///
    /// - [`StalenessCeilingError::Zero`] — `bound` is zero (a zero ceiling
    ///   would demand continuous re-validation).
    /// - [`StalenessCeilingError::AboveHardCap`] — `bound` exceeds
    ///   [`HARD_CAP`](Self::HARD_CAP) (this is the arm that rejects
    ///   `Duration::MAX`).
    pub fn new(bound: Duration) -> Result<Self, StalenessCeilingError> {
        if bound.is_zero() {
            return Err(StalenessCeilingError::Zero);
        }
        if bound > Self::HARD_CAP {
            return Err(StalenessCeilingError::AboveHardCap {
                requested: bound,
                cap: Self::HARD_CAP,
            });
        }
        Ok(Self(bound))
    }

    /// The ceiling value.
    #[must_use]
    pub fn get(self) -> Duration {
        self.0
    }
}

impl Default for StalenessCeiling {
    /// 24 hours — matches the resolver's static re-validation floor cadence so
    /// leased and signal-less-static credentials re-validate on the same
    /// default clock.
    fn default() -> Self {
        Self(Duration::from_hours(24))
    }
}

/// Why [`StalenessCeiling::new`] rejected a bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StalenessCeilingError {
    /// The bound was zero — a ceiling must allow a finite, non-zero interval.
    #[error("staleness ceiling must be non-zero")]
    Zero,
    /// The bound exceeded [`StalenessCeiling::HARD_CAP`] (e.g. `Duration::MAX`).
    #[error("staleness ceiling {requested:?} exceeds the hard cap {cap:?}")]
    AboveHardCap {
        /// The rejected bound.
        requested: Duration,
        /// The hard cap it exceeded.
        cap: Duration,
    },
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

    #[test]
    fn staleness_ceiling_rejects_duration_max() {
        // The real failure mode: a ceiling of `Duration::MAX` would let a
        // leased secret never re-validate. It must be unconstructible.
        assert_eq!(
            StalenessCeiling::new(Duration::MAX),
            Err(StalenessCeilingError::AboveHardCap {
                requested: Duration::MAX,
                cap: StalenessCeiling::HARD_CAP,
            })
        );
    }

    #[test]
    fn staleness_ceiling_rejects_zero() {
        assert_eq!(
            StalenessCeiling::new(Duration::ZERO),
            Err(StalenessCeilingError::Zero)
        );
    }

    #[test]
    fn staleness_ceiling_accepts_in_range_and_round_trips() {
        let c = StalenessCeiling::new(Duration::from_hours(12)).expect("12h is within the cap");
        assert_eq!(c.get(), Duration::from_hours(12));
        // The hard cap itself is inclusive.
        assert!(StalenessCeiling::new(StalenessCeiling::HARD_CAP).is_ok());
    }

    #[test]
    fn staleness_ceiling_default_is_a_day() {
        assert_eq!(StalenessCeiling::default().get(), Duration::from_hours(24));
    }
}
