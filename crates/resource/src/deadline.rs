//! Overflow-safe deadline composition.
//!
//! `Instant + Duration` panics when the sum is unrepresentable, and several
//! budgets reaching these call sites come from operator or author
//! configuration (`PoolConfig::create_timeout`, `Provider::teardown_budget`,
//! slot-hook drain timeouts). Library code must not panic on such input, so
//! every absolute deadline is composed through [`deadline_after`], which
//! saturates to a representable far-future instant instead.

use std::time::{Duration, Instant};

/// Fallback horizon for a budget that is effectively "no deadline".
///
/// Matches the far-future horizon tokio substitutes for an overflowing
/// `timeout`, so an explicit huge budget behaves like tokio's own saturation
/// rather than silently shrinking to a short cap.
pub(crate) const UNBOUNDED_HORIZON: Duration = Duration::from_hours(24 * 365 * 30);

/// Returns `now + budget`, or `now + horizon` when that overflows `Instant`.
///
/// Falls back to `now` only if even the horizon is unrepresentable, which
/// keeps the function total without panicking.
pub(crate) fn deadline_after(now: Instant, budget: Duration, horizon: Duration) -> Instant {
    now.checked_add(budget)
        .or_else(|| now.checked_add(horizon))
        .unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representable_budget_is_added_exactly() {
        let now = Instant::now();
        assert_eq!(
            deadline_after(now, Duration::from_secs(3), UNBOUNDED_HORIZON),
            now + Duration::from_secs(3)
        );
    }

    #[test]
    fn overflowing_budget_saturates_to_the_horizon() {
        let now = Instant::now();
        let horizon = Duration::from_hours(1);
        assert_eq!(deadline_after(now, Duration::MAX, horizon), now + horizon);
    }

    #[test]
    fn unrepresentable_horizon_falls_back_to_now() {
        let now = Instant::now();
        assert_eq!(deadline_after(now, Duration::MAX, Duration::MAX), now);
    }
}
