#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Metrics
//!
//! Unified metric naming and export adapters for the Nebula workflow engine.
//!
//! This crate provides:
//! - **[`naming`]** — typed [`MetricName`] constants with kind and help metadata
//! - **[`snapshot`]** — Prometheus text-format export with `# HELP`, `# TYPE` metadata
//!   and per-bucket histogram output
//! - **[`filter::LabelAllowlist`]** — allowlist that strips high-cardinality label keys
//!   before they reach the registry (prevents cardinality explosion)
//! - **[`prelude`]** — convenience re-exports for common types
//!
//! In-memory primitives (Counter, Gauge, Histogram) remain in `nebula-telemetry`; this crate
//! adds naming convention, Prometheus text export, and label safety guards.

pub mod export;
pub mod filter;
pub mod naming;
/// Convenience re-exports.
pub mod prelude;

pub use export::prometheus::{PrometheusExporter, content_type, snapshot};
pub use filter::LabelAllowlist;
pub use naming::{
    ACTION_DURATION, ACTION_EXECUTIONS, ACTION_FAILURES, ALL_METRICS, EVENTBUS_DROP_RATIO_PPM,
    EVENTBUS_DROPPED, EVENTBUS_SENT, EVENTBUS_SUBSCRIBERS, MetricKind, MetricName,
    RESOURCE_ACQUIRE, RESOURCE_ACQUIRE_WAIT_DURATION, RESOURCE_CIRCUIT_BREAKER_CLOSED,
    RESOURCE_CIRCUIT_BREAKER_OPENED, RESOURCE_CLEANUP, RESOURCE_CONFIG_RELOADED, RESOURCE_CREATE,
    RESOURCE_CREDENTIAL_ROTATED, RESOURCE_ERROR, RESOURCE_HEALTH_STATE, RESOURCE_POOL_EXHAUSTED,
    RESOURCE_POOL_WAITERS, RESOURCE_QUARANTINE, RESOURCE_QUARANTINE_RELEASED, RESOURCE_RELEASE,
    RESOURCE_USAGE_DURATION, WORKFLOW_EXECUTION_DURATION, WORKFLOW_EXECUTIONS_COMPLETED,
    WORKFLOW_EXECUTIONS_FAILED, WORKFLOW_EXECUTIONS_STARTED,
};

// Re-export for convenience so callers can use nebula_metrics::Counter etc.
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
