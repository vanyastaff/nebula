#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-metrics
//!
//! In-memory metric primitives, naming policy, label safety, and Prometheus
//! text-format export for the Nebula workflow engine.
//!
//! ## Purpose
//!
//! Single observability crate covering primitive metric types, label interning,
//! standard `nebula_*` metric names, cardinality safety, and Prometheus export.
//! Per ADR-0046 the formerly separate `nebula-telemetry` primitives layer was
//! absorbed into this crate; the cross-crate boundary was structurally
//! unenforced and caused daily friction. Intra-crate module discipline
//! (`mod` boundaries + `pub`/`pub(crate)`) replaces canon `[L1-§3.10]`.
//!
//! ## Public API
//!
//! - [`naming`] — standard `nebula_*` metric name constants
//! - [`MetricsRegistry`] — concurrent registry for counters, gauges, histograms
//! - [`Counter`], [`Gauge`], [`Histogram`], [`HistogramSnapshot`] — lock-free
//!   metric types backed by atomics
//! - [`LabelInterner`], [`LabelSet`], [`MetricKey`] — interning + composite keys
//! - [`MetricsAdapter`] — adapter using `nebula_*` name constants
//! - [`snapshot`] — Prometheus text-format export
//! - [`filter::LabelAllowlist`] — strips high-cardinality label keys
//! - [`MetricsError`], [`MetricsResult`] — typed error and result alias
//! - [`prelude`] — convenience re-exports

// primitives
pub mod labels;
pub mod registry;
// policy
pub mod adapter;
pub mod filter;
pub mod naming;
// export
pub mod prometheus;
// error
pub mod error;
// prelude
/// Convenience re-exports.
pub mod prelude;

pub use adapter::MetricsAdapter;
pub use error::{MetricKind, MetricsError, MetricsResult};
pub use filter::LabelAllowlist;
pub use labels::{LabelInterner, LabelKey, LabelSet, LabelValue, MetricKey};
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
pub use prometheus::{PrometheusExporter, content_type, snapshot};
pub use registry::{Counter, Gauge, Histogram, HistogramSnapshot, MetricsRegistry};
