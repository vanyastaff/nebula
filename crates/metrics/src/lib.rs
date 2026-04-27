#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-metrics
//!
//! Metric export and label-safety layer for the Nebula workflow engine.
//!
//! ## Purpose
//!
//! Sits on top of `nebula-telemetry` primitives and adds what operators need:
//! consistent `nebula_*` naming, a cardinality guard that strips high-cardinality
//! label keys before they reach the registry, and Prometheus text-format export.
//! Consumers import this crate — it re-exports `Counter`, `Gauge`, `Histogram`, and
//! `MetricsRegistry` from `nebula-telemetry` so only one import is needed.
//! See `crates/metrics/README.md` for the full role description.
//!
//! ## Role
//!
//! **Metric Export and Label-Safety** — sits on top of `nebula-telemetry`; the
//! `/metrics` HTTP scrape endpoint lives in `nebula-api`.
//!
//! ## Public API
//!
//! - [`naming`] — standard `nebula_*` metric name constants
//! - [`TelemetryAdapter`] — adapter over `nebula-telemetry::MetricsRegistry` using `nebula_*` names
//! - [`snapshot`] — Prometheus text-format export with `# HELP`, `# TYPE` metadata and per-bucket
//!   histogram output
//! - [`filter::LabelAllowlist`] — strips high-cardinality label keys (prevents cardinality
//!   explosion)
//! - [`prelude`] — convenience re-exports for common types
//!
//! In-memory primitives (`Counter`, `Gauge`, `Histogram`) remain in `nebula-telemetry`;
//! this crate adds naming convention, a thin adapter, Prometheus text export, and label safety.

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
    NEBULA_CREDENTIAL_EXPIRED_TOTAL, NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS,
    NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL,
    NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL,
    NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL,
    NEBULA_CREDENTIAL_ROTATION_DURATION_SECONDS, NEBULA_CREDENTIAL_ROTATION_FAILURES_TOTAL,
    NEBULA_CREDENTIAL_ROTATIONS_TOTAL, NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL,
    NEBULA_ENGINE_LEASE_CONTENTION_TOTAL, NEBULA_EVENTBUS_DROP_RATIO_PPM, NEBULA_EVENTBUS_DROPPED,
    NEBULA_EVENTBUS_SENT, NEBULA_EVENTBUS_SUBSCRIBERS, NEBULA_RESOURCE_ACQUIRE_ERROR_TOTAL,
    NEBULA_RESOURCE_ACQUIRE_TOTAL, NEBULA_RESOURCE_ACQUIRE_WAIT_DURATION_SECONDS,
    NEBULA_RESOURCE_CLEANUP_TOTAL, NEBULA_RESOURCE_CONFIG_RELOADED_TOTAL,
    NEBULA_RESOURCE_CREATE_TOTAL, NEBULA_RESOURCE_CREDENTIAL_REVOKE_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATED_TOTAL, NEBULA_RESOURCE_CREDENTIAL_ROTATION_ATTEMPTS_TOTAL,
    NEBULA_RESOURCE_CREDENTIAL_ROTATION_DISPATCH_LATENCY_SECONDS, NEBULA_RESOURCE_DESTROY_TOTAL,
    NEBULA_RESOURCE_ERROR_TOTAL, NEBULA_RESOURCE_HEALTH_STATE,
    NEBULA_RESOURCE_POOL_EXHAUSTED_TOTAL, NEBULA_RESOURCE_POOL_WAITERS,
    NEBULA_RESOURCE_QUARANTINE_RELEASED_TOTAL, NEBULA_RESOURCE_QUARANTINE_TOTAL,
    NEBULA_RESOURCE_RELEASE_TOTAL, NEBULA_RESOURCE_USAGE_DURATION_SECONDS,
    NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, NEBULA_WORKFLOW_EXECUTION_DURATION_SECONDS,
    NEBULA_WORKFLOW_EXECUTIONS_COMPLETED_TOTAL, NEBULA_WORKFLOW_EXECUTIONS_FAILED_TOTAL,
    NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL, control_reclaim_outcome,
    engine_lease_contention_reason, refresh_coord_claim_outcome, refresh_coord_coalesced_tier,
    refresh_coord_reclaim_outcome, refresh_coord_sentinel_action, rotation_outcome,
    webhook_signature_failure_reason,
};
// Re-export for convenience so callers can use nebula_metrics::Counter etc.
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
