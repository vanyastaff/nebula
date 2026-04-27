//! Standard metric names for Nebula.
//!
//! Convention: `nebula_<domain>_<metric>_<unit>`.
//! See [docs/crates/metrics/TARGET.md](https://github.com/vanyastaff/nebula/blob/main/docs/crates/metrics/TARGET.md).

// ---------------------------------------------------------------------------
// Workflow (engine)
// ---------------------------------------------------------------------------

/// Counter: workflow executions started.
pub const NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL: &str =
    "nebula_workflow_executions_started_total";

/// Counter: workflow executions completed successfully.
pub const NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL: &str =
    "nebula_workflow_executions_completed_total";

/// Counter: workflow executions failed.
pub const NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL: &str = "nebula_workflow_executions_failed_total";

/// Histogram: workflow execution duration in seconds.
pub const NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS: &str =
    "nebula_workflow_execution_duration_seconds";

/// Counter: engine execution-lease contention events.
///
/// Labeled by `reason` (see [`engine_lease_contention_reason`]). Incremented
/// each time the engine tries to acquire or renew a lease and loses: either
/// another holder is already live (`already_held`) or the current heartbeat
/// detected the lease was taken over / expired (`heartbeat_lost`). Per
/// ADR 0008, `reason=heartbeat_lost` crossing zero is a real multi-runner
/// incident — the engine self-aborted a dispatch rather than produce
/// corrupt checkpoints.
pub const NEBULA_ENGINE_LEASE_CONTENTION_TOTAL: &str = "nebula_engine_lease_contention_total";

/// Reason labels for [`NEBULA_ENGINE_LEASE_CONTENTION_TOTAL`].
///
/// These are the exact static strings emitted as the `reason` label so
/// call sites and tests can compare without stringifying a value twice.
pub mod engine_lease_contention_reason {
    /// Another engine instance holds a live (non-expired) lease — the
    /// caller returned `EngineError::Leased` and did not run.
    pub const ALREADY_HELD: &str = "already_held";
    /// A running engine's heartbeat failed: the lease was stolen or
    /// expired beneath the frontier loop. The engine cancels in-flight
    /// work and refuses to persist further state.
    pub const HEARTBEAT_LOST: &str = "heartbeat_lost";
}

/// Counter: control-queue reclaim sweep outcomes (ADR-0017).
///
/// Labeled by `outcome` (see [`control_reclaim_outcome`]). The
/// `ControlConsumer` reclaim sweep increments this counter by the per-row
/// count for each outcome on every successful sweep — `reclaimed` tracks
/// rows moved `Processing → Pending` for redelivery (a steady climb is a
/// crashed-runner signal), and `exhausted` tracks rows moved
/// `Processing → Failed` once `reclaim_count` reached the budget (any
/// crossing-zero is a genuine incident; cross-reference with
/// [`NEBULA_ENGINE_LEASE_CONTENTION_TOTAL`] heartbeat metrics).
pub const NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL: &str = "nebula_engine_control_reclaim_total";

/// Outcome labels for [`NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL`].
///
/// These are the exact static strings emitted as the `outcome` label so
/// call sites and tests can compare without stringifying a value twice.
/// Only these two values are ever emitted — cardinality hygiene
/// (no `processor_id` label).
pub mod control_reclaim_outcome {
    /// Row transitioned `Processing → Pending` for fresh dispatch.
    pub const RECLAIMED: &str = "reclaimed";
    /// Row transitioned `Processing → Failed` because `reclaim_count`
    /// reached `max_reclaim_count`.
    pub const EXHAUSTED: &str = "exhausted";
}

// ---------------------------------------------------------------------------
// Action (runtime)
// ---------------------------------------------------------------------------

/// Counter: action executions (success + failure).
pub const NEBULA_ACTION_EXECUTIONS_TOTAL: &str = "nebula_action_executions_total";

/// Counter: action failures.
pub const NEBULA_ACTION_FAILURES_TOTAL: &str = "nebula_action_failures_total";

/// Histogram: action execution duration in seconds.
pub const NEBULA_ACTION_DURATION_SECONDS: &str = "nebula_action_duration_seconds";

/// Counter: action dispatches rejected before reaching a handler.
///
/// Labeled by `reason`. Separate from [`NEBULA_ACTION_EXECUTIONS_TOTAL`] so
/// that the duration histogram and execution counter are not skewed by
/// early-rejection paths (trigger / resource / agent / unknown variants).
/// See `runtime::ActionRuntime::run_handler` and
/// [`dispatch_reject_reason`] for the label values.
pub const NEBULA_ACTION_DISPATCH_REJECTED_TOTAL: &str = "nebula_action_dispatch_rejected_total";

/// Reason labels for [`NEBULA_ACTION_DISPATCH_REJECTED_TOTAL`].
///
/// These are the exact static strings emitted as the `reason` label on
/// the dispatch-rejected counter. They are `pub const` so call sites and
/// tests can compare without stringifying a value twice.
pub mod dispatch_reject_reason {
    /// `ActionHandler::Trigger` cannot be executed through `ActionRuntime`.
    pub const TRIGGER_NOT_EXECUTABLE: &str = "trigger_not_executable";
    /// `ActionHandler::Resource` cannot be executed through `ActionRuntime`.
    pub const RESOURCE_NOT_EXECUTABLE: &str = "resource_not_executable";
    /// Unknown `ActionHandler` variant (`#[non_exhaustive]` guard).
    pub const UNKNOWN_VARIANT: &str = "unknown_variant";
}

// ---------------------------------------------------------------------------
// Webhook (api crate — transport-layer signature enforcement)
// ---------------------------------------------------------------------------

/// Counter: webhook requests rejected by the transport-layer signature
/// check (ADR-0022).
///
/// Labeled by `reason` (see [`webhook_signature_failure_reason`]). Low
/// cardinality by design — the label set is exactly three static
/// strings, no per-trigger dimension. Any non-zero value is an
/// operational signal worth dashboarding: a `missing_secret` crossing
/// means an action shipped with a `SignaturePolicy::Required` it did
/// not populate; a `missing` / `invalid` crossing means either a
/// provider is mis-signing or a caller is probing the endpoint.
pub const NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL: &str = "nebula_webhook_signature_failures_total";

/// Reason labels for [`NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`].
///
/// Static strings so call sites and tests compare without stringifying
/// twice. The set is intentionally closed — extending it requires an
/// ADR revision because every added label permanently inflates the
/// cardinality floor for the signature-failure counter.
pub mod webhook_signature_failure_reason {
    /// Signature header absent from the request under a policy that
    /// requires one (`SignaturePolicy::Required` / `Custom` returning
    /// `SignatureOutcome::Missing`).
    pub const MISSING: &str = "missing";
    /// Signature header present but did not match — bad hex / base64,
    /// wrong length, tampered body, or wrong secret
    /// (`SignatureOutcome::Invalid`).
    pub const INVALID: &str = "invalid";
    /// `SignaturePolicy::Required` with an empty secret — an author
    /// shipped the default policy without supplying a secret. Returns
    /// 500 (not 401) because the misconfiguration is on our side.
    pub const MISSING_SECRET: &str = "missing_secret";
}

// ---------------------------------------------------------------------------
// Resource (resource crate)
// ---------------------------------------------------------------------------

/// Counter: resource instances created.
pub const NEBULA_RESOURCE_CREATE_TOTAL: &str = "nebula_resource_create_total";
/// Counter: resource acquisitions.
pub const NEBULA_RESOURCE_ACQUIRE_TOTAL: &str = "nebula_resource_acquire_total";
/// Histogram: wait time before acquisition in seconds.
pub const NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS: &str =
    "nebula_resource_acquire_wait_duration_seconds";
/// Counter: resource releases.
pub const NEBULA_RESOURCE_RELEASE_TOTAL: &str = "nebula_resource_release_total";
/// Histogram: usage duration in seconds.
pub const NEBULA_RESOURCE_USAGE_DURATION_SECONDS: &str = "nebula_resource_usage_duration_seconds";
/// Counter: resource cleanups.
pub const NEBULA_RESOURCE_CLEANUP_TOTAL: &str = "nebula_resource_cleanup_total";
/// Counter: resource instances destroyed (unregistered).
pub const NEBULA_RESOURCE_DESTROY_TOTAL: &str = "nebula_resource_destroy_total";
/// Counter: resource acquire errors.
pub const NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL: &str = "nebula_resource_acquire_error_total";
/// Counter: resource errors.
pub const NEBULA_RESOURCE_ERROR_TOTAL: &str = "nebula_resource_error_total";
/// Gauge: health state (1=healthy, 0.5=degraded/unknown, 0=unhealthy).
pub const NEBULA_RESOURCE_HEALTH_STATE: &str = "nebula_resource_health_state";
/// Counter: pool exhausted events.
pub const NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL: &str = "nebula_resource_pool_exhausted_total";
/// Gauge: number of waiters when pool exhausted.
pub const NEBULA_RESOURCE_POOL_WAITERS: &str = "nebula_resource_pool_waiters";
/// Counter: resources quarantined.
pub const NEBULA_RESOURCE_QUARANTINE_TOTAL: &str = "nebula_resource_quarantine_total";
/// Counter: resources released from quarantine.
pub const NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL: &str =
    "nebula_resource_quarantine_released_total";
/// Counter: config reloads.
pub const NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL: &str = "nebula_resource_config_reloaded_total";
/// Counter: credential rotations applied to a resource pool.
pub const NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL: &str =
    "nebula_resource_credential_rotated_total";
/// Counter: circuit breaker transitioned to open state.
pub const NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL: &str =
    "nebula_resource_circuit_breaker_opened_total";
/// Counter: circuit breaker transitioned to closed state (recovered).
pub const NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL: &str =
    "nebula_resource_circuit_breaker_closed_total";

// ---------------------------------------------------------------------------
// EventBus (generic bus layer)
// ---------------------------------------------------------------------------

/// Gauge: snapshot of sent events for an EventBus instance.
pub const NEBULA_EVENTBUS_SENT: &str = "nebula_eventbus_sent";
/// Gauge: snapshot of dropped events for an EventBus instance.
pub const NEBULA_EVENTBUS_DROPPED: &str = "nebula_eventbus_dropped";
/// Gauge: snapshot of active subscribers for an EventBus instance.
pub const NEBULA_EVENTBUS_SUBSCRIBERS: &str = "nebula_eventbus_subscribers";
/// Gauge: snapshot drop ratio (`0.0..=1.0`) scaled by 1_000_000.
pub const NEBULA_EVENTBUS_DROP_RATIO_PPM: &str = "nebula_eventbus_drop_ratio_ppm";

// ---------------------------------------------------------------------------
// Credential (rotation subsystem)
// ---------------------------------------------------------------------------

/// Counter: total credential rotation attempts.
pub const NEBULA_CREDENTIAL_ROTATIONS_TOTAL: &str = "nebula_credential_rotations_total";
/// Counter: total credential rotation failures.
pub const NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL: &str =
    "nebula_credential_rotation_failures_total";
/// Histogram: credential rotation duration in seconds.
pub const NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS: &str =
    "nebula_credential_rotation_duration_seconds";
/// Gauge: number of active (non-expired) credentials.
pub const NEBULA_CREDENTIAL_ACTIVE_TOTAL: &str = "nebula_credential_active_total";
/// Counter: total credentials that have expired.
pub const NEBULA_CREDENTIAL_EXPIRED_TOTAL: &str = "nebula_credential_expired_total";

// ---------------------------------------------------------------------------
// Credential refresh coordinator (two-tier L1+L2 — sub-spec
// `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` §6)
// ---------------------------------------------------------------------------

/// Counter: L2 claim acquisition outcomes.
///
/// Labeled by `outcome` (see [`refresh_coord_claim_outcome`]). Three
/// closed labels:
///
/// - `acquired` — `RefreshClaimRepo::try_claim` returned `Acquired`; the holder owns the L2 row and
///   may run the refresh closure.
/// - `contended` — `try_claim` returned `Contended`; another replica held the row and the holder
///   slept on backoff (n8n #13088 mitigation lineage).
/// - `exhausted` — backoff retries were exhausted before a claim could be obtained.
///
/// `outcome="exhausted" > 0` is a real production signal; pair with the
/// `holder.contender` log lines to triage. `acquired` rises in lockstep
/// with refresh load.
pub const NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL: &str =
    "nebula_credential_refresh_coord_claims_total";

/// Outcome labels for [`NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL`].
///
/// Static strings so call sites and tests compare without stringifying
/// twice. The set is intentionally closed — adding a label permanently
/// inflates the cardinality floor and requires a sub-spec amendment.
pub mod refresh_coord_claim_outcome {
    /// `RefreshClaimRepo::try_claim` returned `Acquired`.
    pub const ACQUIRED: &str = "acquired";
    /// `RefreshClaimRepo::try_claim` returned `Contended`.
    pub const CONTENDED: &str = "contended";
    /// Backoff retries exhausted before a claim could be obtained.
    pub const EXHAUSTED: &str = "exhausted";
}

/// Counter: refresh calls coalesced rather than running a fresh IdP POST.
///
/// Labeled by `tier` (see [`refresh_coord_coalesced_tier`]). Two closed
/// labels:
///
/// - `l1` — same-process concurrent caller awaited the L1 oneshot waiter and was woken without
///   running its own closure.
/// - `l2` — the post-backoff state recheck found another replica had already refreshed the
///   credential while we slept; `RefreshError::CoalescedByOtherReplica` short-circuits the closure.
///
/// Incremented exactly once per coalesced caller. The ratio
/// `coalesced_total{tier="l2"} / claims_total{outcome="acquired"}` is a
/// directly-actionable n8n #13088-style "near miss" signal.
pub const NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL: &str =
    "nebula_credential_refresh_coord_coalesced_total";

/// Tier labels for [`NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL`].
pub mod refresh_coord_coalesced_tier {
    /// Same-process L1 coalesce — the L1 oneshot waiter resolved.
    pub const L1: &str = "l1";
    /// Cross-replica L2 coalesce — post-backoff state recheck saw a fresh
    /// credential.
    pub const L2: &str = "l2";
}

/// Counter: sentinel events surfaced by the reclaim sweep.
///
/// Labeled by `action` (see [`refresh_coord_sentinel_action`]). Two
/// closed labels:
///
/// - `recorded` — sweep observed an expired `RefreshInFlight` row, recorded the event in
///   `credential_sentinel_events`, and returned `SentinelDecision::Recoverable`.
/// - `reauth_triggered` — same as above, but the rolling-window count crossed `sentinel_threshold`
///   so the sweep emitted `CredentialEvent::ReauthRequired`. Crossing zero is an immediate operator
///   signal (a holder crashed mid-refresh repeatedly within `sentinel_window`).
pub const NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL: &str =
    "nebula_credential_refresh_coord_sentinel_events_total";

/// Action labels for [`NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL`].
pub mod refresh_coord_sentinel_action {
    /// Sentinel event recorded; threshold not yet reached.
    pub const RECORDED: &str = "recorded";
    /// Sentinel threshold crossed; `ReauthRequired` published.
    pub const REAUTH_TRIGGERED: &str = "reauth_triggered";
}

/// Counter: reclaim-sweep outcomes.
///
/// Labeled by `outcome` (see [`refresh_coord_reclaim_outcome`]). Two
/// closed labels:
///
/// - `reclaimed` — sweep deleted at least one expired claim row from `credential_refresh_claims`.
/// - `no_work` — sweep ran and found nothing to reclaim (the steady state for healthy systems).
///
/// The ratio `reclaimed / (reclaimed + no_work)` is the sweep's hit-rate.
/// Sustained high `reclaimed` rate signals a crashed-runner storm; pair
/// with [`NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL`] to
/// distinguish clean timeouts from mid-refresh crashes.
pub const NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL: &str =
    "nebula_credential_refresh_coord_reclaim_sweeps_total";

/// Outcome labels for [`NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL`].
pub mod refresh_coord_reclaim_outcome {
    /// At least one claim row was reclaimed in this sweep.
    pub const RECLAIMED: &str = "reclaimed";
    /// Sweep ran with no expired rows to reclaim.
    pub const NO_WORK: &str = "no_work";
}

/// Histogram: how long a holder owned the L2 claim row.
///
/// Observed in seconds at the moment of release (success path) or
/// reclaim (crash / timeout path). Includes the heartbeat ticks plus
/// the user closure under `refresh_timeout`. P99 should sit below
/// `claim_ttl` by construction (otherwise `validate()` would have
/// rejected the config); P50 should sit near `refresh_timeout` for
/// hot credentials.
pub const NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS: &str =
    "nebula_credential_refresh_coord_hold_duration_seconds";

/// Counter: `reauth_required = true` persistence attempts that exhausted
/// their CAS budget without committing.
///
/// Incremented in `CredentialResolver::resolve_with_refresh` when all
/// 3 CAS attempts to flip `reauth_required` on the credential row lose
/// to a `VersionConflict` and the loop exits without a successful
/// persist. The post-backoff state-recheck on a different replica then
/// reads `reauth_required = false`, re-runs the IdP closure, and
/// produces another `invalid_grant`. Crossing zero is a real signal —
/// pair with [`NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL`]
/// `outcome="acquired"` to see the duplicate-IdP-load amplification.
///
/// Sub-spec §3.6 / I1 best-effort persist: this counter exposes the
/// otherwise-silent failure mode without changing the engine's typed
/// error contract.
pub const NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL: &str =
    "nebula_credential_resolver_reauth_persist_cas_exhausted_total";

// ---------------------------------------------------------------------------
// Cache (memory crate)
// ---------------------------------------------------------------------------

/// Gauge: cache hits snapshot (point-in-time absolute value).
pub const NEBULA_CACHE_HITS: &str = "nebula_cache_hits";
/// Gauge: cache misses snapshot (point-in-time absolute value).
pub const NEBULA_CACHE_MISSES: &str = "nebula_cache_misses";
/// Gauge: cache evictions snapshot (point-in-time absolute value).
pub const NEBULA_CACHE_EVICTIONS: &str = "nebula_cache_evictions";
/// Gauge: current cache size (number of entries).
pub const NEBULA_CACHE_SIZE: &str = "nebula_cache_size";

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::{
        NEBULA_CACHE_EVICTIONS, NEBULA_CACHE_HITS, NEBULA_CACHE_MISSES, NEBULA_CACHE_SIZE,
        NEBULA_CREDENTIAL_ACTIVE_TOTAL, NEBULA_CREDENTIAL_EXPIRED_TOTAL,
        NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
        NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
        NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS,
        NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
        NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
        NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS, NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
        NEBULA_CREDENTIAL_ROTATIONS_TOTAL, NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL,
        NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
        NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
        NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
        NEBULA_RESOURCE_DESTROY_TOTAL, NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
        NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
        NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
        NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    };

    const RESOURCE_METRIC_NAMES: [&str; 16] = [
        NEBULA_RESOURCE_CREATE_TOTAL,
        NEBULA_RESOURCE_ACQUIRE_TOTAL,
        NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
        NEBULA_RESOURCE_RELEASE_TOTAL,
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
        NEBULA_RESOURCE_DESTROY_TOTAL,
        NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL,
    ];

    const RESOURCE_GAUGE_NAMES: [&str; 2] =
        [NEBULA_RESOURCE_HEALTH_STATE, NEBULA_RESOURCE_POOL_WAITERS];

    const RESOURCE_HISTOGRAM_NAMES: [&str; 2] = [
        NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
        NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
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
                let gauge = registry.gauge(metric_name);
                gauge.set(1);
                assert_eq!(gauge.get(), 1);
            } else if RESOURCE_HISTOGRAM_NAMES.contains(&metric_name) {
                let histogram = registry.histogram(metric_name);
                histogram.observe(1.0);
                assert_eq!(histogram.count(), 1);
            } else {
                let counter = registry.counter(metric_name);
                counter.inc();
                assert_eq!(counter.get(), 1);
            }
        }

        assert_eq!(unique.len(), 16);
    }

    const CREDENTIAL_METRIC_NAMES: [&str; 5] = [
        NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
        NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
        NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
        NEBULA_CREDENTIAL_ACTIVE_TOTAL,
        NEBULA_CREDENTIAL_EXPIRED_TOTAL,
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
                let gauge = registry.gauge(metric_name);
                gauge.set(1);
                assert_eq!(gauge.get(), 1);
            } else if metric_name == NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS {
                let histogram = registry.histogram(metric_name);
                histogram.observe(1.0);
                assert_eq!(histogram.count(), 1);
            } else {
                let counter = registry.counter(metric_name);
                counter.inc();
                assert_eq!(counter.get(), 1);
            }
        }
        assert_eq!(unique.len(), 5);
    }

    /// Sub-spec §6 — five refresh-coordinator metrics. The histogram
    /// observes hold-duration in seconds; the four counters carry
    /// closed label sets defined in this module's
    /// `refresh_coord_*` submodules.
    #[test]
    fn credential_refresh_coord_constants_are_accessible_unique_and_registry_safe() {
        let registry = MetricsRegistry::new();
        let mut unique = HashSet::new();
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
                let labels = registry.interner().single("outcome", "ok");
                let histogram = registry.histogram_labeled(metric_name, &labels);
                histogram.observe(0.5);
                assert_eq!(histogram.count(), 1);
            } else {
                // All four refresh-coord counters carry a closed label
                // set; exercise the labeled accessor end-to-end.
                let labels = registry.interner().single("outcome", "acquired");
                let counter = registry.counter_labeled(metric_name, &labels);
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
        use super::{
            refresh_coord_claim_outcome, refresh_coord_coalesced_tier,
            refresh_coord_reclaim_outcome, refresh_coord_sentinel_action,
        };

        let claim = [
            refresh_coord_claim_outcome::ACQUIRED,
            refresh_coord_claim_outcome::CONTENDED,
            refresh_coord_claim_outcome::EXHAUSTED,
        ];
        let claim_set: HashSet<&str> = claim.iter().copied().collect();
        assert_eq!(claim_set.len(), 3, "claim outcome labels must be unique");

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
            refresh_coord_reclaim_outcome::NO_WORK,
        ];
        let reclaim_set: HashSet<&str> = reclaim.iter().copied().collect();
        assert_eq!(
            reclaim_set.len(),
            2,
            "reclaim outcome labels must be unique"
        );
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
            let gauge = registry.gauge(metric_name);
            gauge.set(1);
            assert_eq!(gauge.get(), 1);
        }

        assert_eq!(unique.len(), 4);
    }
}
