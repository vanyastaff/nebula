//! Convenience re-exports for telemetry users.
//!
//! ```rust,ignore
//! use nebula_telemetry::prelude::*;
//! ```

// ── Events ──────────────────────────────────────────────────────────────────
pub use crate::EventFilter;
pub use crate::PublishOutcome;
pub use crate::ScopedEvent;
pub use crate::SubscriptionScope;
pub use crate::event::{EventBus, EventSubscriber, ExecutionEvent, ScopedSubscriber};

// ── Metrics ─────────────────────────────────────────────────────────────────
pub use crate::metrics::{Counter, Gauge, Histogram, MetricsRegistry, NoopMetricsRegistry};

// ── Service ─────────────────────────────────────────────────────────────────
pub use crate::service::{NoopTelemetry, TelemetryService};

// ── Trace ───────────────────────────────────────────────────────────────────
pub use crate::trace::{
    CallBody, CallPayload, CallRecord, CallStatus, DropReason, NoopRecorder, Recorder,
    ResourceUsageRecord,
};
