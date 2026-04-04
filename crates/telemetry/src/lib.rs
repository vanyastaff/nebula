#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Telemetry
//!
//! In-memory metrics primitives for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`MetricsRegistry`] — concurrent registry for counters, gauges, and histograms
//! - [`Counter`], [`Gauge`], [`Histogram`] — lock-free metric types backed by atomics
//! - [`LabelInterner`] / [`LabelSet`] — `lasso`-backed string interning for
//!   label keys and values, enabling zero-copy metric dimensions
//! - [`TelemetryError`] — unified error type for the telemetry subsystem
//!
//! ## Metric naming
//!
//! Use the `nebula_` prefix for metric names (e.g. `nebula_executions_total`,
//! `nebula_action_duration_seconds`) to avoid collisions and support
//! future export (Prometheus/OTLP). See the crate ROADMAP for the full convention.

pub mod error;
pub mod labels;
pub mod metrics;

pub use error::{TelemetryError, TelemetryResult};
pub use labels::{LabelInterner, LabelSet, MetricKey};
pub use metrics::{Counter, Gauge, Histogram, MetricsRegistry};
