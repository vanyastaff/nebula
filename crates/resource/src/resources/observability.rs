//! Observability resource implementations
//!
//! This module provides re-exports for observability resources:
//! - **`LoggerResource`**: Structured logging with nebula-log integration
//! - **`MetricsResource`**: Prometheus metrics collection and export
//! - **`TracerResource`**: OpenTelemetry distributed tracing
//!
//! # Features
//!
//! - `metrics` - Enable Prometheus metrics export
//! - `tracing` - Enable OpenTelemetry tracing
//!
//! # Example
//!
//! ```rust,no_run
//! use nebula_resource::resources::observability::{MetricsResource, MetricsConfig};
//!
//! let metrics_resource = MetricsResource;
//! let config = MetricsConfig {
//!     endpoint: "0.0.0.0:9090".to_string(),
//!     namespace: "nebula".to_string(),
//!     ..Default::default()
//! };
//! ```

// Re-export logger
pub use crate::resources::logger::{LoggerConfig, LoggerInstance, LoggerResource};

// Re-export metrics
pub use crate::resources::metrics::{MetricsConfig, MetricsInstance, MetricsResource};

// Re-export tracer
pub use crate::resources::tracer::{TracerConfig, TracerInstance, TracerResource};
