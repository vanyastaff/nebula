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

// ---------------------------------------------------------------------------
// Action (runtime)
// ---------------------------------------------------------------------------

/// Counter: action executions (success + failure).
pub const NEBULA_ACTION_EXECUTIONS_TOTAL: &str = "nebula_action_executions_total";

/// Counter: action failures.
pub const NEBULA_ACTION_FAILURES_TOTAL: &str = "nebula_action_failures_total";

/// Histogram: action execution duration in seconds.
pub const NEBULA_ACTION_DURATION_SECONDS: &str = "nebula_action_duration_seconds";

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
// Cache (memory crate)
// ---------------------------------------------------------------------------

/// Counter: total cache hits.
pub const NEBULA_CACHE_HITS_TOTAL: &str = "nebula_cache_hits_total";
/// Counter: total cache misses.
pub const NEBULA_CACHE_MISSES_TOTAL: &str = "nebula_cache_misses_total";
/// Counter: total cache evictions.
pub const NEBULA_CACHE_EVICTIONS_TOTAL: &str = "nebula_cache_evictions_total";
/// Gauge: current cache size (number of entries).
pub const NEBULA_CACHE_SIZE: &str = "nebula_cache_size";

// ---------------------------------------------------------------------------
// Legacy names (for backward compatibility during migration)
// ---------------------------------------------------------------------------

/// Legacy: use [`NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL`].
pub const LEGACY_EXECUTIONS_STARTED_TOTAL: &str = "executions_started_total";

/// Legacy: use [`NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL`].
pub const LEGACY_EXECUTIONS_COMPLETED_TOTAL: &str = "executions_completed_total";

/// Legacy: use [`NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL`].
pub const LEGACY_EXECUTIONS_FAILED_TOTAL: &str = "executions_failed_total";

/// Legacy: use [`NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS`].
pub const LEGACY_EXECUTION_DURATION_SECONDS: &str = "execution_duration_seconds";

/// Legacy: use [`NEBULA_ACTION_EXECUTIONS_TOTAL`].
pub const LEGACY_ACTIONS_EXECUTED_TOTAL: &str = "actions_executed_total";

/// Legacy: use [`NEBULA_ACTION_FAILURES_TOTAL`].
pub const LEGACY_ACTIONS_FAILED_TOTAL: &str = "actions_failed_total";

/// Legacy: use [`NEBULA_ACTION_DURATION_SECONDS`].
pub const LEGACY_ACTION_DURATION_SECONDS: &str = "action_duration_seconds";

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::{
        NEBULA_CACHE_EVICTIONS_TOTAL, NEBULA_CACHE_HITS_TOTAL, NEBULA_CACHE_MISSES_TOTAL,
        NEBULA_CACHE_SIZE, NEBULA_CREDENTIAL_ACTIVE_TOTAL, NEBULA_CREDENTIAL_EXPIRED_TOTAL,
        NEBULA_CREDENTIAL_ROTATIONS_TOTAL, NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
        NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL, NEBULA_RESOURCE_ACQUIRE_TOTAL,
        NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS, NEBULA_RESOURCE_CLEANUP_TOTAL,
        NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL, NEBULA_RESOURCE_CREATE_TOTAL,
        NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL, NEBULA_RESOURCE_ERROR_TOTAL,
        NEBULA_RESOURCE_HEALTH_STATE, NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL,
        NEBULA_RESOURCE_POOL_WAITERS, NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL,
        NEBULA_RESOURCE_QUARANTINE_TOTAL, NEBULA_RESOURCE_RELEASE_TOTAL,
        NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    };

    const RESOURCE_METRIC_NAMES: [&str; 14] = [
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

            let counter = registry.counter(metric_name);
            counter.inc();
            assert_eq!(counter.get(), 1);
        }

        assert_eq!(unique.len(), 14);
    }

    const CREDENTIAL_METRIC_NAMES: [&str; 5] = [
        NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
        NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
        NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
        NEBULA_CREDENTIAL_ACTIVE_TOTAL,
        NEBULA_CREDENTIAL_EXPIRED_TOTAL,
    ];

    const CACHE_METRIC_NAMES: [&str; 4] = [
        NEBULA_CACHE_HITS_TOTAL,
        NEBULA_CACHE_MISSES_TOTAL,
        NEBULA_CACHE_EVICTIONS_TOTAL,
        NEBULA_CACHE_SIZE,
    ];

    #[test]
    fn credential_constants_are_accessible_unique_and_registry_safe() {
        let registry = MetricsRegistry::new();
        let mut unique = HashSet::new();
        for metric_name in CREDENTIAL_METRIC_NAMES {
            assert!(!metric_name.is_empty());
            assert!(metric_name.starts_with("nebula_credential_"));
            assert!(metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'));
            assert!(unique.insert(metric_name));
            let counter = registry.counter(metric_name);
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
        assert_eq!(unique.len(), 5);
    }

    #[test]
    fn cache_constants_are_accessible_unique_and_registry_safe() {
        let registry = MetricsRegistry::new();
        let mut unique = HashSet::new();
        for metric_name in CACHE_METRIC_NAMES {
            assert!(!metric_name.is_empty());
            assert!(metric_name.starts_with("nebula_cache_"));
            assert!(metric_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'));
            assert!(unique.insert(metric_name));
            let counter = registry.counter(metric_name);
            counter.inc();
            assert_eq!(counter.get(), 1);
        }
        assert_eq!(unique.len(), 4);
    }
}
