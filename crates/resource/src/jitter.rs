//! Shared jitter helper for spreading a nominal duration to avoid lockstep
//! expiry across a fleet of identically-configured entities.
//!
//! Two consumers share this one algorithm at different spreads:
//!
//! - [`recovery::gate`](crate::recovery::gate) — "equal jitter" (`spread =
//!   0.5`) on the recovery-gate retry backoff, so a fleet of gates that
//!   failed at the same instant do not all re-probe in lockstep.
//! - [`runtime::pool`](crate::runtime::pool) — a small attenuation (`spread`
//!   a few percent) on `max_lifetime`, HikariCP-style, so a warmup burst of
//!   pool entries created together does not all expire on the same
//!   maintenance tick.

use std::time::Duration;

/// Spreads `nominal` uniformly over `[nominal * (1 - spread), nominal]`.
///
/// `spread` is clamped to `[0.0, 1.0]`. `spread == 0.0` is a no-op
/// (`nominal` unchanged, no entropy draw); `spread == 0.5` is "equal
/// jitter" (recovery-gate backoff spread); a small `spread` (e.g. `0.05`) is
/// HikariCP's `maxLifetime` attenuation — it only trims up to that fraction
/// off the configured duration, keeping the eviction band close to `nominal`
/// while still de-synchronizing entities that started together. A zero
/// `nominal` stays zero regardless of `spread`.
///
/// Entropy is [`std::hash::RandomState`] — a per-thread OS-seeded key mixed
/// with a per-instance counter (std draws fresh OS randomness only once per
/// thread, then derives each `RandomState::new()` from that seed), not a
/// fresh OS random draw on every call. That is uniform enough for a jitter
/// spread and needs no `rand` dependency for the cold paths that call this
/// (one draw per failed recovery attempt, or per newly-created pool entry).
pub(crate) fn apply_jitter(nominal: Duration, spread: f64) -> Duration {
    use std::hash::{BuildHasher, RandomState};

    debug_assert!(
        spread.is_finite(),
        "jitter spread must be finite, got {spread}"
    );
    let spread = spread.clamp(0.0, 1.0);
    // Floor rather than round: `Duration::mul_f64` rounds to the nearest
    // representable nanosecond, which can round a sub-nanosecond span (e.g.
    // 1ns * 0.5 = 0.5ns) *up* to 1ns — turning a should-be no-op edge case
    // into a coin-flip between the nominal value and one nanosecond less.
    // Flooring matches the original integer-division truncation semantics
    // (`nominal / 2` for the equal-jitter `spread == 0.5` case) and keeps
    // every sub-integer-nanosecond product a deterministic no-op.
    let span_nanos_f64 = (nominal.as_nanos() as f64 * spread).floor();
    let span_nanos = u64::try_from(span_nanos_f64 as u128).unwrap_or(u64::MAX);
    if span_nanos == 0 {
        return nominal;
    }
    // Uniform-enough draw in [0, span]: SipHash output of a fresh
    // randomly-seeded RandomState. Modulo bias over a 64-bit draw is
    // negligible for a jitter spread.
    let draw_nanos = RandomState::new().hash_one(0u64) % (span_nanos + 1);
    // `draw <= span <= nominal`, so this never saturates; `saturating_sub`
    // states the no-underflow intent without a panic path.
    nominal.saturating_sub(Duration::from_nanos(draw_nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_nominal_stays_zero() {
        assert_eq!(apply_jitter(Duration::ZERO, 0.5), Duration::ZERO);
        assert_eq!(apply_jitter(Duration::ZERO, 0.05), Duration::ZERO);
    }

    #[test]
    fn zero_spread_is_a_no_op() {
        let nominal = Duration::from_secs(30);
        for _ in 0..100 {
            assert_eq!(apply_jitter(nominal, 0.0), nominal);
        }
    }

    #[test]
    fn equal_jitter_band_is_half_to_nominal() {
        let nominal = Duration::from_secs(10);
        for _ in 0..1_000 {
            let jittered = apply_jitter(nominal, 0.5);
            assert!(jittered >= nominal / 2, "{jittered:?} below the half bound");
            assert!(jittered <= nominal, "{jittered:?} above nominal");
        }
    }

    #[test]
    fn small_spread_band_matches_hikaricp_attenuation() {
        // [0.95 * L, L] — jitter proportional to lifetime.
        let nominal = Duration::from_mins(30);
        let lower_bound = nominal.mul_f64(0.95);
        for _ in 0..1_000 {
            let jittered = apply_jitter(nominal, 0.05);
            assert!(
                jittered >= lower_bound,
                "{jittered:?} below the 0.95*L bound"
            );
            assert!(jittered <= nominal, "{jittered:?} above nominal");
        }
    }

    #[test]
    fn sub_nanosecond_span_is_a_no_op() {
        // A 1ns nominal at 5% spread rounds its span down to 0ns — must not
        // panic and must return the nominal unchanged (mirrors the original
        // `apply_equal_jitter(Duration::from_nanos(1))` edge case).
        assert_eq!(
            apply_jitter(Duration::from_nanos(1), 0.05),
            Duration::from_nanos(1)
        );
    }
}
