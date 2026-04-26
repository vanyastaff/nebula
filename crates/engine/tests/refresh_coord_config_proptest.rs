//! Property tests for `RefreshCoordConfig::validate` (sub-spec §3.5).
//!
//! Verifies the interlocking invariants:
//!
//! - `heartbeat_interval × 3 ≤ claim_ttl`
//! - `refresh_timeout + 2 × heartbeat_interval ≤ claim_ttl`
//! - `reclaim_sweep_interval ≤ claim_ttl`
//!
//! by exhaustively probing the parameter space and asserting that
//! `validate()` is `Ok` iff every invariant holds. Boundary case
//! `heartbeat × 3 == claim_ttl` is allowed — see `RefreshCoordConfig`
//! docs.

use std::time::Duration;

use nebula_engine::credential::refresh::RefreshCoordConfig;
use proptest::prelude::*;

#[test]
fn default_config_validates() {
    // CI assertion that the shipped defaults are consistent.
    assert!(
        RefreshCoordConfig::default().validate().is_ok(),
        "RefreshCoordConfig::default() must satisfy §3.5 invariants"
    );
}

proptest! {
    /// `validate()` returns `Ok(())` exactly when each of the three
    /// invariants holds. Whatever input we throw at it, the predicate
    /// derived from the spec text must agree with the implementation.
    #[test]
    fn validate_passes_iff_all_invariants_hold(
        ttl_secs in 5u64..300,
        hb_secs in 1u64..100,
        refresh_secs in 1u64..200,
        sweep_secs in 1u64..400,
    ) {
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(ttl_secs),
            heartbeat_interval: Duration::from_secs(hb_secs),
            refresh_timeout: Duration::from_secs(refresh_secs),
            reclaim_sweep_interval: Duration::from_secs(sweep_secs),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };

        let valid = cfg.validate().is_ok();
        let invariants_hold = hb_secs.saturating_mul(3) <= ttl_secs
            && refresh_secs.saturating_add(hb_secs.saturating_mul(2)) <= ttl_secs
            && sweep_secs <= ttl_secs;

        prop_assert_eq!(valid, invariants_hold);
    }

    /// Heartbeat-too-slow case: holding the other two invariants while
    /// pushing `heartbeat_interval × 3 > claim_ttl` (strict overshoot)
    /// must fail.
    #[test]
    fn heartbeat_too_slow_fails(
        ttl_secs in 30u64..300,
        // bump 1.. so hb * 3 strictly exceeds ttl
        bump in 1u64..50,
    ) {
        let hb_secs = ttl_secs / 3 + bump;
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(ttl_secs),
            heartbeat_interval: Duration::from_secs(hb_secs),
            // Pick refresh_timeout small enough not to also trip the
            // other invariant — otherwise the result is still Err but
            // for the "wrong" reason.
            refresh_timeout: Duration::from_secs(1),
            reclaim_sweep_interval: Duration::from_secs(1),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_hours(1),
        };
        prop_assert!(cfg.validate().is_err());
    }
}
