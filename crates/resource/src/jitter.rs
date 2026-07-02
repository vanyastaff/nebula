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
    let nominal_nanos = nominal.as_nanos();
    // Floor rather than round: `Duration::mul_f64` rounds to the nearest
    // representable nanosecond, which can round a sub-nanosecond span (e.g.
    // 1ns * 0.5 = 0.5ns) *up* to 1ns — turning a should-be no-op edge case
    // into a coin-flip between the nominal value and one nanosecond less.
    // Flooring matches the original integer-division truncation semantics
    // (`nominal / 2` for the equal-jitter `spread == 0.5` case) and keeps
    // every sub-integer-nanosecond product a deterministic no-op.
    let span_nanos_f64 = (nominal_nanos as f64 * spread).floor();
    // Stay in `u128` end to end: `nominal_nanos` (a `Duration::as_nanos()`)
    // can be as large as ~1.8e28 for `Duration::MAX`, so a `u64` span would
    // saturate to `u64::MAX` there and make the modulus below (`span + 1`)
    // wrap to `0` — a guaranteed divide-by-zero panic in both debug and
    // release, not just an overflow. `u128` has enough headroom that
    // `span_nanos + 1` can never wrap for any real `Duration`. The `as u128`
    // cast saturates (never UB) on a NaN/negative/out-of-range float; the
    // trailing `.min(nominal_nanos)` additionally guards against the f64
    // multiplication rounding a huge `nominal` fractionally *above* its own
    // true value, so the span can never exceed the nominal it was derived
    // from.
    let span_nanos = (span_nanos_f64 as u128).min(nominal_nanos);
    if span_nanos == 0 {
        return nominal;
    }
    // Uniform-enough draw in [0, span]: SipHash output of a fresh
    // randomly-seeded RandomState, widened to `u128` before the modulo (the
    // draw itself is always `<= u64::MAX`, so the `u64::try_from` below
    // always succeeds — `unwrap_or` is a defensive, unreachable-in-practice
    // fallback, not a real truncation path). Modulo bias over a 64-bit draw
    // folded into a much larger `u128` span is negligible for a jitter
    // spread.
    let draw_nanos_u128 = u128::from(RandomState::new().hash_one(0u64)) % (span_nanos + 1);
    let draw_nanos = u64::try_from(draw_nanos_u128).unwrap_or(u64::MAX);
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

    /// `Duration::MAX.as_nanos()` (~1.8e28) is the regression case for the
    /// `u64`-space `span_nanos + 1` overflow: a saturated `u64::MAX` span
    /// made the modulus wrap to `0`, a guaranteed divide-by-zero panic in
    /// both debug and release. Doing the modulo math in `u128` must survive
    /// this without panicking, and the result must still land in
    /// `[nominal * (1 - spread), nominal]`.
    #[test]
    fn u64_max_scale_nominal_does_not_overflow_or_panic() {
        let nominal = Duration::MAX;
        for spread in [0.0, 0.05, 0.5, 1.0] {
            let jittered = apply_jitter(nominal, spread);
            assert!(
                jittered <= nominal,
                "spread {spread}: {jittered:?} must never exceed nominal"
            );
        }
    }

    #[test]
    fn nominal_at_u64_max_seconds_equal_jitter_stays_in_band() {
        // `u64::MAX` seconds is itself already far past `u64::MAX`
        // nanoseconds — squarely in the range the `u64`-space overflow
        // regression could panic on.
        let nominal = Duration::from_secs(u64::MAX);
        for _ in 0..100 {
            let jittered = apply_jitter(nominal, 0.5);
            assert!(jittered >= nominal / 2, "{jittered:?} below the half bound");
            assert!(jittered <= nominal, "{jittered:?} above nominal");
        }
    }
}
