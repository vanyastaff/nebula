//! Observability hooks for resilience patterns
//!
//! This module provides integration points for distributed tracing,
//! metrics export, and structured logging.

pub mod hooks;
pub mod spans;

pub use hooks::{
    LogLevel, LoggingHook, MetricsHook, ObservabilityHook, ObservabilityHooks, PatternEvent,
};
pub use spans::{SpanGuard, create_span, record_error, record_success};
