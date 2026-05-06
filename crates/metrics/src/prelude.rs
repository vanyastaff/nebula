//! Convenience re-exports for metrics users.
//!
//! ```rust,ignore
//! use nebula_metrics::prelude::*;
//! ```

// ── Adapter ─────────────────────────────────────────────────────────────────
// ── Metric Types ──────────────────────────────────────────────────────────
pub use crate::registry::{Counter, Gauge, Histogram, MetricsRegistry};

pub use crate::adapter::MetricsAdapter;
// ── Export ───────────────────────────────────────────────────────────────────
pub use crate::prometheus::{PrometheusExporter, content_type, snapshot};
// ── Filter ───────────────────────────────────────────────────────────────────
pub use crate::filter::LabelAllowlist;
