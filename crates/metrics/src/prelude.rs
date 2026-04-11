//! Convenience re-exports for metrics users.
//!
//! ```rust,ignore
//! use nebula_metrics::prelude::*;
//! ```

// ── Adapter ─────────────────────────────────────────────────────────────────
// ── Metric Types (from nebula-telemetry) ────────────────────────────────────
pub use nebula_telemetry::metrics::{Counter, Gauge, Histogram, MetricsRegistry};

pub use crate::adapter::TelemetryAdapter;
// ── Export ───────────────────────────────────────────────────────────────────
pub use crate::export::prometheus::{PrometheusExporter, content_type, snapshot};
// ── Filter ───────────────────────────────────────────────────────────────────
pub use crate::filter::LabelAllowlist;
