#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Telemetry
//!
//! Event bus, metrics, and observability for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`EventBus`] — broadcast-based event distribution (backed by [`nebula_eventbus`])
//! - [`ExecutionEvent`] — execution lifecycle events
//! - [`TelemetryService`] trait — pluggable telemetry backend
//! - [`NoopTelemetry`] — no-op implementation for testing/MVP
//! - [`prelude`] — convenience re-exports for common types
//!
//! Events are **projections**, not the source of truth; the execution store
//! remains the single source of truth.
//!
//! ## Metric naming
//!
//! Use the `nebula_` prefix for metric names (e.g. `nebula_executions_total`,
//! `nebula_action_duration_seconds`) to avoid collisions and support
//! future export (Prometheus/OTLP). See the crate ROADMAP for the full convention.

pub mod event;
pub mod metrics;
/// Convenience re-exports.
pub mod prelude;
pub mod service;
pub mod trace;

pub use event::{EventBus, EventSubscriber, ExecutionEvent, ScopedSubscriber};
pub use metrics::{Counter, Gauge, Histogram, MetricsRegistry, NoopMetricsRegistry};
pub use nebula_eventbus::{EventFilter, PublishOutcome, ScopedEvent, SubscriptionScope};
pub use service::{NoopTelemetry, TelemetryService};
pub use trace::{
    CallBody, CallPayload, CallRecord, CallStatus, DropReason, NoopRecorder, Recorder,
    ResourceUsageRecord,
};
