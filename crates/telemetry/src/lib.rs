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
//! - [`ProductionTelemetry`] — full-featured implementation with event bus, metrics,
//!   and [`BufferedRecorder`]
//! - [`NoopTelemetry`] — no-op implementation for testing/MVP
//! - [`TraceContext`] — W3C trace context propagation (trace ID + span ID + sampling)
//! - [`BufferedRecorder`] — background-buffered resource call recording with
//!   pluggable [`RecordSink`]
//!
//! Events are **projections**, not the source of truth; the execution store
//! remains the single source of truth.
//!
//! ## Metric naming
//!
//! Use the `nebula_` prefix for metric names (e.g. `nebula_executions_total`,
//! `nebula_action_duration_seconds`) to avoid collisions and support
//! future export (Prometheus/OTLP). See the crate ROADMAP for the full convention.

pub mod context;
pub mod event;
pub mod metrics;
pub mod recorder;
pub mod service;
pub mod trace;

pub use context::{SpanId, TraceContext, TraceContextError, TraceId};
pub use event::{EventBus, EventSubscriber, ExecutionEvent, ScopedSubscriber};
pub use metrics::{Counter, Gauge, Histogram, MetricsRegistry, NoopMetricsRegistry};
pub use nebula_eventbus::{EventFilter, PublishOutcome, ScopedEvent, SubscriptionScope};
pub use recorder::{BufferedRecorder, BufferedRecorderConfig, LogSink, RecordEntry, RecordSink};
pub use service::{
    NoopTelemetry, ProductionTelemetry, ProductionTelemetryBuilder, TelemetryService,
};
pub use trace::{
    CallBody, CallPayload, CallRecord, CallStatus, DropReason, NoopRecorder, Recorder,
    ResourceUsageRecord,
};

// ── Prelude ──────────────────────────────────────────────────────────────────

/// Common types for star-import (`use nebula_telemetry::prelude::*`).
pub mod prelude {
    pub use crate::context::{SpanId, TraceContext, TraceId};
    pub use crate::event::{EventBus, ExecutionEvent};
    pub use crate::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
    pub use crate::recorder::{
        BufferedRecorder, BufferedRecorderConfig, LogSink, RecordEntry, RecordSink,
    };
    pub use crate::service::{
        NoopTelemetry, ProductionTelemetry, ProductionTelemetryBuilder, TelemetryService,
    };
    pub use crate::trace::{CallRecord, NoopRecorder, Recorder, ResourceUsageRecord};
}
