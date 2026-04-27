//! Pre-bound metric handles for the refresh coordinator (sub-spec §6).
//!
//! All handles are constructed once at coordinator-build time and live
//! for the coordinator's lifetime, so the hot path (per-`refresh_coalesced`
//! call) pays only an atomic increment / observation — no string interning,
//! no map lookup, no allocation.
//!
//! The five metrics declared in
//! `nebula_metrics::naming::NEBULA_CREDENTIAL_REFRESH_COORD_*` are bound
//! to their closed label sets here:
//!
//! - `claims_total{outcome=acquired|contended|exhausted}`
//! - `coalesced_total{tier=l1|l2}`
//! - `sentinel_events_total{action=recorded|reauth_triggered}`
//! - `reclaim_sweeps_total{outcome=reclaimed|no_work}`
//! - `hold_duration_seconds` (histogram, no labels)
//!
//! Production composition threads the engine-shared registry via
//! [`RefreshCoordMetrics::with_registry`]. Tests / single-replica desktop
//! mode use [`RefreshCoordMetrics::for_tests`] (gated on `cfg(test)` /
//! `feature = "test-util"`) which constructs handles backed by a fresh
//! private registry — production code must always go through the engine
//! registry so a scraper actually observes the series, which is why the
//! `Default` impl has been intentionally removed.

use nebula_metrics::{
    Counter, Histogram, MetricsRegistry, NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS,
    NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL, refresh_coord_claim_outcome,
    refresh_coord_coalesced_tier, refresh_coord_reclaim_outcome, refresh_coord_sentinel_action,
};

/// Pre-bound handles for the five refresh-coordinator metrics declared
/// in sub-spec §6. Cheaply cloneable (each handle is `Arc<...>` under
/// the hood).
#[derive(Clone, Debug)]
pub struct RefreshCoordMetrics {
    // claims_total
    pub(crate) claims_acquired: Counter,
    pub(crate) claims_contended: Counter,
    pub(crate) claims_exhausted: Counter,
    // coalesced_total
    pub(crate) coalesced_l1: Counter,
    pub(crate) coalesced_l2: Counter,
    // sentinel_events_total
    pub(crate) sentinel_recorded: Counter,
    pub(crate) sentinel_reauth_triggered: Counter,
    // reclaim_sweeps_total
    pub(crate) reclaim_reclaimed: Counter,
    pub(crate) reclaim_no_work: Counter,
    // hold_duration_seconds
    pub(crate) hold_duration: Histogram,
}

impl RefreshCoordMetrics {
    /// Build pre-bound handles against the given registry.
    pub fn with_registry(registry: &MetricsRegistry) -> Self {
        let interner = registry.interner();

        let claim_label = |val: &str| interner.single("outcome", val);
        let coalesced_label = |val: &str| interner.single("tier", val);
        let sentinel_label = |val: &str| interner.single("action", val);
        let reclaim_label = |val: &str| interner.single("outcome", val);

        Self {
            claims_acquired: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
                &claim_label(refresh_coord_claim_outcome::ACQUIRED),
            ),
            claims_contended: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
                &claim_label(refresh_coord_claim_outcome::CONTENDED),
            ),
            claims_exhausted: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
                &claim_label(refresh_coord_claim_outcome::EXHAUSTED),
            ),
            coalesced_l1: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
                &coalesced_label(refresh_coord_coalesced_tier::L1),
            ),
            coalesced_l2: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
                &coalesced_label(refresh_coord_coalesced_tier::L2),
            ),
            sentinel_recorded: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
                &sentinel_label(refresh_coord_sentinel_action::RECORDED),
            ),
            sentinel_reauth_triggered: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
                &sentinel_label(refresh_coord_sentinel_action::REAUTH_TRIGGERED),
            ),
            reclaim_reclaimed: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
                &reclaim_label(refresh_coord_reclaim_outcome::RECLAIMED),
            ),
            reclaim_no_work: registry.counter_labeled(
                NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
                &reclaim_label(refresh_coord_reclaim_outcome::NO_WORK),
            ),
            hold_duration: registry
                .histogram(NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS),
        }
    }

    /// Construct handles backed by a fresh private registry — tests and
    /// single-replica desktop mode only.
    ///
    /// Production composition MUST use [`Self::with_registry`] with the
    /// engine-shared registry so a scraper actually observes the series.
    /// Removing the public `Default` impl in wave 2 of the PR-583 review
    /// closed a foot-gun where a top-level composer could silently
    /// publish to a private registry no scraper sees.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn for_tests() -> Self {
        Self::with_registry(&MetricsRegistry::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_tests_handles_are_independent_per_label() {
        let metrics = RefreshCoordMetrics::for_tests();
        metrics.claims_acquired.inc();
        assert_eq!(metrics.claims_acquired.get(), 1);
        assert_eq!(metrics.claims_contended.get(), 0);
        assert_eq!(metrics.claims_exhausted.get(), 0);

        metrics.coalesced_l1.inc();
        metrics.coalesced_l2.inc_by(2);
        assert_eq!(metrics.coalesced_l1.get(), 1);
        assert_eq!(metrics.coalesced_l2.get(), 2);
    }

    #[test]
    fn handles_share_state_with_registry() {
        let registry = MetricsRegistry::new();
        let m1 = RefreshCoordMetrics::with_registry(&registry);
        let m2 = RefreshCoordMetrics::with_registry(&registry);
        // Same registry → same underlying counter.
        m1.claims_acquired.inc();
        assert_eq!(m2.claims_acquired.get(), 1);
    }

    #[test]
    fn hold_duration_records_to_histogram() {
        let metrics = RefreshCoordMetrics::for_tests();
        metrics.hold_duration.observe(0.123);
        metrics.hold_duration.observe(0.456);
        assert_eq!(metrics.hold_duration.count(), 2);
    }
}
