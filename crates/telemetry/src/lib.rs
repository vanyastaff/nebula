#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! # Nebula Telemetry
//!
//! Event bus, metrics, and observability for the Nebula workflow engine.
//!
//! This crate provides:
//! - [`EventBus`] -- broadcast-based event distribution
//! - [`ExecutionEvent`] -- execution lifecycle events
//! - [`TelemetryService`] trait -- pluggable telemetry backend
//! - [`NoopTelemetry`] -- no-op implementation for testing/MVP
//!
//! Events are **projections**, not the source of truth.
//! The [`ports::ExecutionRepo`] is the single source of truth.

pub mod event;
pub mod metrics;
pub mod service;

pub use event::{EventBus, EventSubscriber, ExecutionEvent};
pub use metrics::{Counter, Gauge, Histogram, MetricsRegistry, NoopMetricsRegistry};
pub use service::{NoopTelemetry, TelemetryService};
