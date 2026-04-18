//! Feature gating tests for `ActionResult::Retry`.
//!
//! Per canon §11.2 / §4.5 (operational honesty), the `Retry` variant is
//! hidden behind the default-off `unstable-retry-scheduler` feature flag until
//! the engine retry scheduler lands end-to-end (#290). These tests pin the
//! contract:
//!
//! - When the feature is enabled, the variant constructs and round-trips.
//! - When the feature is disabled, the variant must not be reachable; the `compile_fail` doc test
//!   on `ActionResult` covers that direction by demonstrating that a default-feature consumer
//!   cannot name `ActionResult::Retry`.
//!
//! The CI matrix runs this file with and without the feature; the default run
//! proves the gated arm is truly excluded.

#[cfg(feature = "unstable-retry-scheduler")]
mod enabled {
    use std::time::Duration;

    use nebula_action::ActionResult;

    #[test]
    fn retry_variant_constructs_under_feature() {
        let r: ActionResult<()> = ActionResult::Retry {
            after: Duration::from_secs(5),
            reason: "upstream not ready".into(),
        };
        assert!(r.is_retry());
    }

    #[test]
    fn retry_variant_survives_map_output() {
        let r: ActionResult<i32> = ActionResult::Retry {
            after: Duration::from_millis(750),
            reason: "rate limit".into(),
        };
        let mapped = r.map_output(|n| n * 2);
        match mapped {
            ActionResult::Retry { after, reason } => {
                assert_eq!(after, Duration::from_millis(750));
                assert_eq!(reason, "rate limit");
            },
            _ => panic!("expected Retry after map_output"),
        }
    }
}

#[cfg(not(feature = "unstable-retry-scheduler"))]
mod disabled {
    use nebula_action::ActionResult;

    #[test]
    fn other_variants_still_work_without_feature() {
        // Spot-check: the rest of `ActionResult` is unaffected by the gate.
        // (The negative `compile_fail` check lives in the crate-level doc
        // test so it runs only on default features.)
        let r: ActionResult<i32> = ActionResult::success(42);
        assert!(r.is_success());
    }
}
