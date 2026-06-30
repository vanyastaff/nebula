//! Convenience re-exports for metrics users.
//!
//! ```
//! use nebula_metrics::prelude::*;
//!
//! let registry = MetricsRegistry::new();
//! let requests = registry.counter("requests_total")?;
//! requests.inc();
//! requests.inc();
//! assert_eq!(requests.get(), 2);
//! # Ok::<(), nebula_metrics::MetricsError>(())
//! ```

pub use crate::counter::Counter;
pub use crate::eventbus::record_eventbus_stats;
pub use crate::filter::LabelAllowlist;
pub use crate::gauge::Gauge;
pub use crate::histogram::{Histogram, HistogramSnapshot};
pub use crate::labels::{LabelInterner, LabelKey, LabelSet, MetricKey};
pub use crate::prometheus::{PrometheusExporter, content_type, snapshot};
pub use crate::registry::MetricsRegistry;
