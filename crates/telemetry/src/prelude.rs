//! Convenience re-exports for telemetry users.
//!
//! ```rust,ignore
//! use nebula_telemetry::prelude::*;
//! ```

// ── Service ─────────────────────────────────────────────────────────────────
pub use crate::service::{NoopTelemetry, TelemetryService};

// ── Events ──────────────────────────────────────────────────────────────────
pub use crate::event::{EventBus, ExecutionEvent};

// ── Metrics ─────────────────────────────────────────────────────────────────
pub use crate::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

// ── Recording ───────────────────────────────────────────────────────────────
pub use crate::recorder::{NoopRecorder, Recorder};
