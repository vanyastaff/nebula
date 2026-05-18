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
//! The formerly separate `nebula-telemetry` primitives layer was absorbed into
//! this crate; intra-crate module discipline (`mod` boundaries + `pub`/`pub(crate)`)
//! replaces a cross-crate boundary that was structurally unenforced.
//!
//! ## Public API
//!
//! - [`naming`] — standard `nebula_*` metric name constants
//! - [`MetricsRegistry`] — concurrent registry for counters, gauges, histograms
//! - [`Counter`], [`Gauge`], [`Histogram`], [`HistogramSnapshot`] — lock-free
//!   metric types backed by atomics
//! - [`LabelInterner`], [`LabelSet`], [`MetricKey`] — interning + composite keys
//! - [`record_eventbus_stats`] — free function recording an
//!   [`nebula_eventbus::EventBusStats`] snapshot into the four
//!   `NEBULA_EVENTBUS_*` gauges
//! - [`snapshot`] — Prometheus text-format export
//! - [`LabelAllowlist`] — strips high-cardinality label keys
//! - [`MetricsError`], [`MetricsResult`] — typed error and result alias
//! - [`prelude`] — convenience re-exports

// primitives
mod counter;
mod gauge;
mod histogram;
mod labels;
mod registry;
// policy
mod filter;
pub mod naming;
// export
mod prometheus;
// instrumentation
mod eventbus;
// error
mod error;

pub mod prelude;

pub use counter::Counter;
pub use error::{MetricKind, MetricsError, MetricsResult};
pub use eventbus::record_eventbus_stats;
pub use filter::LabelAllowlist;
pub use gauge::Gauge;
pub use histogram::{Histogram, HistogramSnapshot};
pub use labels::{LabelInterner, LabelKey, LabelSet, LabelValue, MetricKey};
pub use naming::*;
pub use prometheus::{PrometheusExporter, content_type, snapshot};
pub use registry::MetricsRegistry;
