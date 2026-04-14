#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Metrics
//!
//! Unified metric naming and export adapters for the Nebula workflow engine.
//!
//! This crate provides:
//! - **[`naming`]** — standard `nebula_*` metric name constants
//! - **[`TelemetryAdapter`]** — adapter over `nebula-telemetry::MetricsRegistry` that records using
//!   those names
//! - **[`snapshot`]** — Prometheus text-format export with `# HELP`, `# TYPE` metadata and
//!   per-bucket histogram output
//! - **[`filter::LabelAllowlist`]** — allowlist that strips high-cardinality label keys before they
//!   reach the registry (prevents cardinality explosion)
//! - **[`prelude`]** — convenience re-exports for common types
//!
//! In-memory primitives (Counter, Gauge, Histogram) remain in `nebula-telemetry`; this crate
//! adds naming convention, a thin adapter, Prometheus text export, and label safety guards.

pub mod adapter;
pub mod export;
pub mod filter;
pub mod naming;
/// Convenience re-exports.
pub mod prelude;

pub use adapter::TelemetryAdapter;
pub use export::prometheus::{PrometheusExporter, content_type, snapshot};
pub use filter::LabelAllowlist;
pub use naming::{
    NEBULA_ACTION_DISPATCH_REJECTED_TOTAL, NEBULA_ACTION_DURATION_SECONDS,
    NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_FAILURES_TOTAL, NEBULA_CACHE_EVICTIONS,
    NEBULA_CACHE_HITS, NEBULA_CACHE_MISSES, NEBULA_CACHE_SIZE, NEBULA_CREDENTIAL_ACTIVE_TOTAL,
    NEBULA_CREDENTIAL_EXPIRED_TOTAL, NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS,
    NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL, NEBULA_CREDENTIAL_ROTATIONS_TOTAL,
    NEBULA_EVENTBUS_DROP_RATIO_PPM, NEBULA_EVENTBUS_DROPPED, NEBULA_EVENTBUS_SENT,
    NEBULA_EVENTBUS_SUBSCRIBERS, NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL,
    NEBULA_RESOURCE_DESTROY_TOTAL, NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS, NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};
// Re-export for convenience so callers can use nebula_metrics::Counter etc.
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
