#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Metrics
//!
//! Unified metric naming and export adapters for the Nebula workflow engine.
//!
//! This crate provides:
//! - **[`naming`]** — standard `nebula_*` metric name constants
//! - **[`TelemetryAdapter`]** — adapter over `nebula-telemetry::MetricsRegistry` that records
//!   using those names
//!
//! In-memory primitives (Counter, Gauge, Histogram) remain in `nebula-telemetry`; this crate
//! adds naming convention, a thin adapter, and Prometheus text export.

pub mod adapter;
pub mod export;
pub mod naming;

pub use adapter::TelemetryAdapter;
pub use export::prometheus::{content_type, snapshot, PrometheusExporter};
pub use naming::{
    NEBULA_ACTION_DURATION_SECONDS,
    NEBULA_ACTION_EXECUTIONS_TOTAL,
    NEBULA_ACTION_FAILURES_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CLEANUP_TOTAL,
    NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL,
    NEBULA_RESOURCE_ERROR_TOTAL,
    NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL,
    NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL,
    NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL,
    NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL,
};

// Re-export for convenience so callers can use nebula_metrics::Counter etc.
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
