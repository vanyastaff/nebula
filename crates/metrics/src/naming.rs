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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nebula_telemetry::metrics::MetricsRegistry;

    use super::{
        NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
        NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
        NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
        NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
        NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
        NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
        NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
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

    use super::*;

    fn assert_naming_convention(names: &[&str], prefix: &str) {
        let mut unique = HashSet::new();
        for name in names {
            assert!(!name.is_empty(), "constant must not be empty");
            assert!(name.starts_with(prefix), "{name} must start with {prefix}");
            assert!(
                name.chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                "{name} contains invalid chars"
            );
            assert!(unique.insert(*name), "{name} is duplicated");
        }
    }

    #[test]
    fn workflow_constants_follow_naming_convention() {
        assert_naming_convention(
            &[
                NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
                NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
                NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
                NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
            ],
            "nebula_workflow_",
        );
    }

    #[test]
    fn action_constants_follow_naming_convention() {
        assert_naming_convention(
            &[
                NEBULA_ACTION_EXECUTIONS_TOTAL,
                NEBULA_ACTION_FAILURES_TOTAL,
                NEBULA_ACTION_DURATION_SECONDS,
            ],
            "nebula_action_",
        );
    }

    #[test]
    fn eventbus_constants_follow_naming_convention() {
        assert_naming_convention(
            &[
                NEBULA_EVENTBUS_SENT,
                NEBULA_EVENTBUS_DROPPED,
                NEBULA_EVENTBUS_SUBSCRIBERS,
                NEBULA_EVENTBUS_DROP_RATIO_PPM,
            ],
            "nebula_eventbus_",
        );
    }

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
}
