//! Convenience re-exports for metrics users.
//!
//! ```rust,ignore
//! use nebula_metrics::prelude::*;
//! ```

// ── Adapter ─────────────────────────────────────────────────────────────────
pub use crate::adapter::TelemetryAdapter;

// ── Filter ───────────────────────────────────────────────────────────────────
pub use crate::filter::LabelAllowlist;

// ── Export ───────────────────────────────────────────────────────────────────
pub use crate::export::prometheus::{PrometheusExporter, content_type, snapshot};

// ── Metric Types (from nebula-telemetry) ────────────────────────────────────
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};
