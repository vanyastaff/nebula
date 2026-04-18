#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # nebula-telemetry
//!
//! In-memory metric primitives for the Nebula workflow engine.
//!
//! ## Purpose
//!
//! Every crate that records metrics needs the same thread-safe primitive building
//! blocks: counter, gauge, histogram, and zero-copy label dimensions. This crate
//! provides exactly those primitives and nothing else. Naming conventions, export
//! adapters, and Prometheus text generation are deliberately out of scope — they
//! live in `nebula-metrics` one layer above. See `crates/telemetry/README.md` for
//! the full role description.
//!
//! ## Role
//!
//! **Metric Primitives** — cross-cutting infrastructure; only `nebula-error` as an
//! intra-workspace dependency. Consumers should generally import `nebula-metrics`,
//! which re-exports these types.
//!
//! ## Public API
//!
//! - [`MetricsRegistry`] — concurrent registry for counters, gauges, and histograms
//! - [`Counter`], [`Gauge`], [`Histogram`] — lock-free metric types backed by atomics
//! - [`LabelInterner`] / [`LabelSet`] — `lasso`-backed string interning for label keys and values,
//!   enabling zero-copy metric dimensions
//! - [`MetricKey`] — typed metric identity (name + label set)
//! - [`TelemetryError`], [`TelemetryResult`] — typed error and result alias

pub mod error;
pub mod labels;
pub mod metrics;

pub use error::{TelemetryError, TelemetryResult};
pub use labels::{LabelInterner, LabelSet, MetricKey};
pub use metrics::{Counter, Gauge, Histogram, MetricsRegistry};
