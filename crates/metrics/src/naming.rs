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
