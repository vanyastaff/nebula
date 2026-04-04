#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Telemetry
//!
//! Event bus, metrics, and observability for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`EventBus`] — broadcast-based event distribution (backed by [`nebula_eventbus`])
//! - [`ExecutionEvent`] — execution lifecycle events with optional [`TraceContext`]
//! - [`TelemetryService`] trait — pluggable telemetry backend
//! - [`ProductionTelemetry`] — full-featured implementation with event bus and metrics
//! - [`NoopTelemetry`] — no-op implementation for testing/MVP
//! - [`TraceContext`] — W3C trace context propagation (trace ID + span ID + sampling)
//! - [`LabelInterner`] / [`LabelSet`] — `lasso`-backed string interning for
//!   label keys and values, enabling zero-copy metric dimensions
//! - [`TelemetryError`] — unified error type for the telemetry subsystem
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

pub mod error;
pub mod event;
pub mod labels;
pub mod metrics;
/// Convenience re-exports.
pub mod prelude;
pub mod service;
pub mod trace;

pub use error::{TelemetryError, TelemetryResult};
pub use event::{EventBus, EventSubscriber, ExecutionEvent, ScopedSubscriber};
pub use labels::{LabelInterner, LabelSet, MetricKey};
pub use metrics::{Counter, Gauge, Histogram, MetricsRegistry};
pub use service::{
    NoopTelemetry, ProductionTelemetry, ProductionTelemetryBuilder, TelemetryService,
};
pub use trace::{SpanId, TraceContext, TraceContextError, TraceId};
