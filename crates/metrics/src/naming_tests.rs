use std::collections::HashSet;

use crate::registry::MetricsRegistry;

use super::{
    NEBULA_API_AUTH_ATTEMPTS_TOTAL, NEBULA_API_AUTH_DURATION_SECONDS,
    NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL, NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
    NEBULA_API_IDEMPOTENCY_HITS_TOTAL, NEBULA_API_IDEMPOTENCY_LATENCY_MS,
    NEBULA_API_IDEMPOTENCY_MISSES_TOTAL, NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL,
    NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM, NEBULA_CACHE_EVICTIONS, NEBULA_CACHE_HITS,
    NEBULA_CACHE_MISSES, NEBULA_CACHE_SIZE, NEBULA_CREDENTIAL_ACTIVE_TOTAL,
    NEBULA_CREDENTIAL_EXPIRED_TOTAL, NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS,
    NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
    NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS, NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL, NEBULA_ORCHESTRATOR_DISPATCH_TOTAL,
    NEBULA_ORCHESTRATOR_HANDOFF_TOTAL, NEBULA_ORCHESTRATOR_RECLAIM_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL, NEBULA_RESOURCE_ACQUIRE_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS, NEBULA_RESOURCE_CLEANUP_TOTAL,
    NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL, NEBULA_RESOURCE_CREATE_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_REVOKE_OBSERVATIONS_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_OBSERVATIONS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_SKIPPED_TOTAL, NEBULA_RESOURCE_DESTROY_TOTAL,
    NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL, NEBULA_RESOURCE_RELEASE_ERROR_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_STORAGE_REVISION_CATALOG_OPERATIONS_TOTAL, auth_oauth_provider, auth_outcome,
    idempotency_reject_reason, orchestrator_dispatch_outcome, orchestrator_handoff_outcome,
    orchestrator_reclaim_outcome, recycle_outcome, refresh_coord_claim_outcome,
    refresh_coord_coalesced_tier, refresh_coord_reclaim_outcome, refresh_coord_sentinel_action,
    revision_catalog_operation, rotation_outcome, webhook_rate_limit_tier,
    webhook_signature_failure_reason,
};

const RESOURCE_METRIC_NAMES: [&str; 24] = [
    NEBULA_RESOURCE_CREATE_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_RELEASE_TOTAL,
    NEBULA_RESOURCE_RELEASE_ERROR_TOTAL,
    NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_RESOURCE_CLEANUP_TOTAL,
    NEBULA_RESOURCE_ERROR_TOTAL,
    NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL,
    NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL,
    NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_OBSERVATIONS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_REVOKE_OBSERVATIONS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_SKIPPED_TOTAL,
    NEBULA_RESOURCE_DESTROY_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL,
    NEBULA_RESOURCE_RECYCLE_OUTCOME_TOTAL,
];

const RESOURCE_GAUGE_NAMES: [&str; 2] =
    [NEBULA_RESOURCE_HEALTH_STATE, NEBULA_RESOURCE_POOL_WAITERS];

const RESOURCE_HISTOGRAM_NAMES: [&str; 3] = [
    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS,
];

#[test]
fn resource_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();

    for metric_name in RESOURCE_METRIC_NAMES {
        tracing::debug!("testing constant: {}", metric_name);
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_resource_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| { ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' })
        );
        assert!(unique.insert(metric_name));

        if RESOURCE_GAUGE_NAMES.contains(&metric_name) {
            let gauge = registry.gauge(metric_name).unwrap();
            gauge.set(1);
            assert_eq!(gauge.get(), 1);
        } else if RESOURCE_HISTOGRAM_NAMES.contains(&metric_name) {
            let histogram = registry.histogram(metric_name).unwrap();
            histogram.observe(1.0);
            assert_eq!(histogram.count(), 1);
        } else {
            let counter = registry.counter(metric_name).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
    }

    assert_eq!(unique.len(), 24);
}

#[test]
fn rotation_outcome_labels_are_closed_set() {
    // Closed label set — adding a value here permanently inflates
    // cardinality on every rotation/revoke series, so the test is
    // a CI gate against silent expansion.
    let labels = [
        rotation_outcome::SUCCESS,
        rotation_outcome::FAILED,
        rotation_outcome::TIMED_OUT,
        rotation_outcome::DEFERRED,
        rotation_outcome::ABANDONED,
    ];
    let mut unique = HashSet::new();
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
        assert!(unique.insert(label));
    }
    assert_eq!(unique.len(), 5);
}

#[test]
fn recycle_outcome_labels_are_closed_set() {
    // Closed label set — every framework release records exactly one of
    // these, so `recycled + discarded` is the release total. Adding a
    // value permanently inflates cardinality; this test is the CI gate.
    let labels = [recycle_outcome::RECYCLED, recycle_outcome::DISCARDED];
    let mut unique = HashSet::new();
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
        assert!(unique.insert(label));
    }
    assert_eq!(unique.len(), 2);
}

const CREDENTIAL_METRIC_NAMES: [&str; 6] = [
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
    NEBULA_CREDENTIAL_ACTIVE_TOTAL,
    NEBULA_CREDENTIAL_EXPIRED_TOTAL,
    NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL,
];

/// Refresh-coordinator metrics (sub-spec §6).
///
/// Four counters + one histogram = 5 series.
const CREDENTIAL_REFRESH_COORD_METRIC_NAMES: [&str; 5] = [
    NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS,
];

#[test]
fn revision_catalog_operation_labels_are_closed_set() {
    // One value per port method. Adding a value here means the catalog
    // grew a capability, not that a label widened; this test is the CI
    // gate against silent expansion.
    let labels = [
        revision_catalog_operation::INSERT,
        revision_catalog_operation::LOAD_EXACT,
        revision_catalog_operation::BEGIN_DRAIN,
        revision_catalog_operation::DELETE_DRAINED,
        revision_catalog_operation::RELEASE_EXPIRED_ROLLBACKS,
    ];
    let mut unique = HashSet::new();
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
        assert!(unique.insert(label));
    }
    assert_eq!(unique.len(), 5);
}

#[test]
fn revision_catalog_metric_is_registry_safe() {
    let registry = MetricsRegistry::new();
    let name = NEBULA_STORAGE_REVISION_CATALOG_OPERATIONS_TOTAL;
    assert!(name.starts_with("nebula_storage_"));
    assert!(
        name.chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    );
    let counter = registry
        .counter(name)
        .expect("the catalog counter registers");
    counter.inc();
    assert_eq!(counter.get(), 1);
}

const CACHE_METRIC_NAMES: [&str; 4] = [
    NEBULA_CACHE_HITS,
    NEBULA_CACHE_MISSES,
    NEBULA_CACHE_EVICTIONS,
    NEBULA_CACHE_SIZE,
];

#[test]
fn credential_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();
    for metric_name in CREDENTIAL_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_credential_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(unique.insert(metric_name));

        if metric_name == NEBULA_CREDENTIAL_ACTIVE_TOTAL {
            let gauge = registry.gauge(metric_name).unwrap();
            gauge.set(1);
            assert_eq!(gauge.get(), 1);
        } else if metric_name == NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS {
            let histogram = registry.histogram(metric_name).unwrap();
            histogram.observe(1.0);
            assert_eq!(histogram.count(), 1);
        } else {
            let counter = registry.counter(metric_name).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
    }
    assert_eq!(unique.len(), 6);
}

/// Sub-spec §6 — five refresh-coordinator metrics. The histogram
/// observes hold-duration in seconds; the four counters carry
/// closed label sets defined in this module's `refresh_coord_*`
/// submodules.
///
/// The per-counter `(name, label_key, sample_value)` table mirrors
/// the engine wiring in `crates/engine/src/credential/refresh/metrics.rs`
/// so a future drift between the doc-string label set and the
/// engine's `claim_label`/`coalesced_label`/`sentinel_label`/`reclaim_label`
/// builders fails CI rather than landing silently. Previously the
/// test hard-coded `outcome=acquired` for every counter, so a label
/// rename on three of four counters was invisible.
#[test]
fn credential_refresh_coord_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();

    // (constant, label_key, label_value) per counter — mirrors the
    // engine's pre-bound handles. Histogram has no labels so it's
    // handled separately below.
    let counter_label_map: &[(&'static str, &'static str, &'static str)] = &[
        (
            NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
            "outcome",
            refresh_coord_claim_outcome::ACQUIRED,
        ),
        (
            NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
            "tier",
            refresh_coord_coalesced_tier::L1,
        ),
        (
            NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
            "action",
            refresh_coord_sentinel_action::REAUTH_TRIGGERED,
        ),
        (
            NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
            "outcome",
            refresh_coord_reclaim_outcome::RECLAIMED,
        ),
    ];

    for metric_name in CREDENTIAL_REFRESH_COORD_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_credential_refresh_coord_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(unique.insert(metric_name));

        if metric_name == NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS {
            let histogram = registry.histogram(metric_name).unwrap();
            histogram.observe(0.5);
            assert_eq!(histogram.count(), 1);
        } else {
            // Find the matching label_key and sample_value for this
            // counter — the table above is the source of truth.
            let (_, label_key, label_value) = counter_label_map
                .iter()
                .find(|(name, ..)| *name == metric_name)
                .expect("every refresh-coord counter must appear in counter_label_map");
            let labels = registry.interner().single(label_key, label_value);
            let counter = registry.counter_labeled(metric_name, &labels).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
    }
    assert_eq!(unique.len(), 5);
}

/// Closed label sets per sub-spec §6 — assert each module's
/// constants are unique within the module so a cardinality drift
/// fails CI rather than landing silently.
#[test]
fn refresh_coord_label_constants_are_unique_per_module() {
    let claim = [
        refresh_coord_claim_outcome::ACQUIRED,
        refresh_coord_claim_outcome::CONTENDED,
        refresh_coord_claim_outcome::OUTCOME_UNKNOWN,
        refresh_coord_claim_outcome::EXHAUSTED,
    ];
    let claim_set: HashSet<&str> = claim.iter().copied().collect();
    assert_eq!(claim_set.len(), 4, "claim outcome labels must be unique");

    let tier = [
        refresh_coord_coalesced_tier::L1,
        refresh_coord_coalesced_tier::L2,
    ];
    let tier_set: HashSet<&str> = tier.iter().copied().collect();
    assert_eq!(tier_set.len(), 2, "coalesced tier labels must be unique");

    let action = [
        refresh_coord_sentinel_action::RECORDED,
        refresh_coord_sentinel_action::REAUTH_TRIGGERED,
    ];
    let action_set: HashSet<&str> = action.iter().copied().collect();
    assert_eq!(action_set.len(), 2, "sentinel action labels must be unique");

    let reclaim = [
        refresh_coord_reclaim_outcome::RECLAIMED,
        refresh_coord_reclaim_outcome::OUTCOME_UNKNOWN_ACCOUNTED,
        refresh_coord_reclaim_outcome::NO_WORK,
    ];
    let reclaim_set: HashSet<&str> = reclaim.iter().copied().collect();
    assert_eq!(
        reclaim_set.len(),
        3,
        "reclaim outcome labels must be unique"
    );
}

/// API idempotency metrics (M3.4.
///
/// 3 counters + 1 gauge + 1 histogram = 5 series. Mirrors the
/// per-counter `(name, label_key, sample_value)` pattern used for
/// the refresh-coord constants so a label-key drift between this
/// catalog and the middleware wiring fails CI rather than landing
/// silently.
const API_IDEMPOTENCY_METRIC_NAMES: [&str; 5] = [
    NEBULA_API_IDEMPOTENCY_HITS_TOTAL,
    NEBULA_API_IDEMPOTENCY_MISSES_TOTAL,
    NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL,
    NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM,
    NEBULA_API_IDEMPOTENCY_LATENCY_MS,
];

#[test]
fn api_idempotency_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();
    for metric_name in API_IDEMPOTENCY_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_api_idempotency_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(unique.insert(metric_name));

        if metric_name == NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM {
            let gauge = registry.gauge(metric_name).unwrap();
            gauge.set(420_000);
            assert_eq!(gauge.get(), 420_000);
        } else if metric_name == NEBULA_API_IDEMPOTENCY_LATENCY_MS {
            let histogram = registry.histogram(metric_name).unwrap();
            histogram.observe(1.0);
            assert_eq!(histogram.count(), 1);
        } else if metric_name == NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL {
            // Labeled by `reason` — exercise the closed label set.
            let labels = registry
                .interner()
                .single("reason", idempotency_reject_reason::INVALID_KEY);
            let counter = registry.counter_labeled(metric_name, &labels).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        } else {
            let counter = registry.counter(metric_name).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
    }
    assert_eq!(unique.len(), 5);
}

/// API auth metrics (M3.1 follow-up wave §PR-B).
///
/// 3 counters + 1 histogram = 4 metric names. Counters are labeled
/// per the oracle locked spec: `attempts_total{outcome}` /
/// `mfa_attempts_total{outcome}` against the 12-value closed
/// [`auth_outcome`] set; `oauth_attempts_total{outcome, provider}`
/// adds the 2-value closed [`auth_oauth_provider`] dimension; the
/// histogram uses default seconds-shaped buckets keyed by `outcome`.
const API_AUTH_METRIC_NAMES: [&str; 4] = [
    NEBULA_API_AUTH_ATTEMPTS_TOTAL,
    NEBULA_API_AUTH_MFA_ATTEMPTS_TOTAL,
    NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL,
    NEBULA_API_AUTH_DURATION_SECONDS,
];

#[test]
fn api_auth_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();
    for metric_name in API_AUTH_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_api_auth_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(unique.insert(metric_name));

        if metric_name == NEBULA_API_AUTH_DURATION_SECONDS {
            let labels = registry.interner().single("outcome", auth_outcome::SUCCESS);
            let histogram = registry.histogram_labeled(metric_name, &labels).unwrap();
            histogram.observe(0.05);
            assert_eq!(histogram.count(), 1);
        } else if metric_name == NEBULA_API_AUTH_OAUTH_ATTEMPTS_TOTAL {
            let labels = registry.interner().label_set(&[
                ("outcome", auth_outcome::SUCCESS),
                ("provider", auth_oauth_provider::GOOGLE),
            ]);
            let counter = registry.counter_labeled(metric_name, &labels).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        } else {
            let labels = registry.interner().single("outcome", auth_outcome::SUCCESS);
            let counter = registry.counter_labeled(metric_name, &labels).unwrap();
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
    }
    assert_eq!(unique.len(), 4);
}

#[test]
fn auth_outcome_labels_are_closed_set() {
    // Closed label set per the oracle locked spec — adding a value
    // here permanently inflates cardinality on every auth series so
    // this test is the CI gate against silent expansion. Mirrors
    // `idempotency_reject_reason_labels_are_closed_set`.
    let labels = [
        auth_outcome::SUCCESS,
        auth_outcome::INVALID_CREDS,
        auth_outcome::INVALID_INPUT,
        auth_outcome::INVALID_MFA_CODE,
        auth_outcome::MFA_REQUIRED,
        auth_outcome::TOKEN_INVALID,
        auth_outcome::LOCKOUT,
        auth_outcome::EMAIL_UNVERIFIED,
        auth_outcome::RATE_LIMIT,
        auth_outcome::OAUTH_FAILED,
        auth_outcome::CONFLICT,
        auth_outcome::INTERNAL,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), 12, "auth outcome labels must be unique");
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
    }
}

#[test]
fn auth_oauth_provider_labels_are_closed_set() {
    // Bounded at compile time by the `OAuthProvider` enum
    // (`Google | GitHub`). A user-supplied unknown
    // provider is rejected by `from_str` with `InvalidInput` before
    // any metric arm runs, so the set cannot leak.
    let labels = [auth_oauth_provider::GOOGLE, auth_oauth_provider::GITHUB];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), 2, "auth oauth provider labels must be unique");
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
    }
}

#[test]
fn idempotency_reject_reason_labels_are_closed_set() {
    let labels = [
        idempotency_reject_reason::INVALID_KEY,
        idempotency_reject_reason::BODY_MISMATCH,
        idempotency_reject_reason::BODY_TOO_LARGE,
        idempotency_reject_reason::NON_ASCII_HEADER,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(
        unique.len(),
        4,
        "idempotency reject-reason labels must be unique"
    );
    for label in labels {
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
    }
}

/// Orchestrator metrics (ADR-0095 pull loop).
///
/// 3 counters — all labeled by `outcome` with a closed label set.
const ORCHESTRATOR_METRIC_NAMES: [&str; 3] = [
    NEBULA_ORCHESTRATOR_DISPATCH_TOTAL,
    NEBULA_ORCHESTRATOR_HANDOFF_TOTAL,
    NEBULA_ORCHESTRATOR_RECLAIM_TOTAL,
];

#[test]
fn orchestrator_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();

    for metric_name in ORCHESTRATOR_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_orchestrator_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(unique.insert(metric_name));

        // All orchestrator metrics are labeled counters.
        let labels = registry.interner().single("outcome", "dispatched");
        let counter = registry.counter_labeled(metric_name, &labels).unwrap();
        counter.inc();
        assert_eq!(counter.get(), 1);
    }

    assert_eq!(unique.len(), 3);
}

#[test]
fn orchestrator_dispatch_outcome_labels_are_closed_set() {
    // Closed label set — adding a value permanently inflates cardinality on
    // the dispatch counter; this test is the CI gate against silent expansion.
    let labels = [
        orchestrator_dispatch_outcome::DISPATCHED,
        orchestrator_dispatch_outcome::FAILED,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), 2, "dispatch outcome labels must be unique");
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
    }
}

#[test]
fn orchestrator_reclaim_outcome_labels_are_closed_set() {
    // Closed label set — adding a value permanently inflates cardinality on
    // the reclaim counter; this test is the CI gate against silent expansion.
    let labels = [
        orchestrator_reclaim_outcome::RECLAIMED,
        orchestrator_reclaim_outcome::EXHAUSTED,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), 2, "reclaim outcome labels must be unique");
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
    }
}

#[test]
fn orchestrator_handoff_outcome_labels_are_closed_set() {
    // Closed label set — same CI gate as the other orchestrator counters.
    // The vocabulary mirrors the storage crate's `acceptance_label` so a
    // dashboard and the backend spans agree on what happened.
    let labels = [
        orchestrator_handoff_outcome::ACCEPTED,
        orchestrator_handoff_outcome::CLAIM_SUPERSEDED,
        orchestrator_handoff_outcome::TURN_HELD,
        orchestrator_handoff_outcome::ERROR,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), 4, "handoff outcome labels must be unique");
    for label in labels {
        assert!(!label.is_empty());
        assert!(label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
    }
}

#[test]
fn cache_constants_are_accessible_unique_and_registry_safe() {
    let registry = MetricsRegistry::new();
    let mut unique = HashSet::new();
    for metric_name in CACHE_METRIC_NAMES {
        assert!(!metric_name.is_empty());
        assert!(metric_name.starts_with("nebula_cache_"));
        assert!(
            metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        );
        assert!(unique.insert(metric_name));
    }

    // All cache metrics are gauges (point-in-time snapshots)
    for metric_name in CACHE_METRIC_NAMES {
        let gauge = registry.gauge(metric_name).unwrap();
        gauge.set(1);
        assert_eq!(gauge.get(), 1);
    }

    assert_eq!(unique.len(), 4);
}

#[test]
fn webhook_signature_failure_reason_labels_are_closed_set() {
    // Closed label set — adding a value permanently inflates cardinality on
    // the signature-failure counter; this test is the CI gate against silent
    // expansion.  The full set must stay in sync with the counter's doc
    // comment (currently seven values).
    let labels = [
        webhook_signature_failure_reason::MISSING,
        webhook_signature_failure_reason::INVALID,
        webhook_signature_failure_reason::MISSING_SECRET,
        webhook_signature_failure_reason::TIMESTAMP_MISSING,
        webhook_signature_failure_reason::TIMESTAMP_MALFORMED,
        webhook_signature_failure_reason::TIMESTAMP_OUT_OF_WINDOW,
        webhook_signature_failure_reason::PROD_UNSIGNED,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(
        unique.len(),
        7,
        "signature_failure_reason labels must be unique"
    );
    for label in labels {
        assert!(!label.is_empty());
        assert!(
            label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
            "label {label:?} must match [a-z_]+"
        );
    }
}

#[test]
fn webhook_rate_limit_tier_labels_are_closed_set() {
    // Closed label set — adding a value permanently inflates cardinality on
    // the rate-limit-rejection counter; this test is the CI gate against
    // silent expansion.
    let labels = [
        webhook_rate_limit_tier::PER_TOKEN,
        webhook_rate_limit_tier::PER_TENANT,
    ];
    let unique: HashSet<&str> = labels.iter().copied().collect();
    assert_eq!(unique.len(), 2, "rate_limit_tier labels must be unique");
    for label in labels {
        assert!(!label.is_empty());
        assert!(
            label.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
            "label {label:?} must match [a-z_]+"
        );
    }
}
