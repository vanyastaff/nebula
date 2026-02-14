//! Observability hooks for resilience patterns
//!
//! This module provides integration points for distributed tracing,
//! metrics export, and structured logging.
//!
//! # Type-Safe Observability
//!
//! The module includes advanced type-safe patterns:
//!
//! - **Event categories** for compile-time event classification
//! - **Const generic metrics** for zero-cost dimension tracking
//! - **Pattern spans** with compile-time pattern validation
//!
//! ```rust,ignore
//! use nebula_resilience::observability::{
//!     Event, RetryEventCategory, metrics,
//! };
//!
//! // Create event with category
//! let event = Event::<RetryEventCategory>::new("api_call")
//!     .with_duration(Duration::from_millis(100));
//!
//! // Create metric
//! let metric = metrics::operation_histogram("latency", "api", "get", 50.0);
//! ```

pub mod hooks;
pub mod spans;

// Original exports
pub use hooks::{
    LogLevel, LoggingHook, MetricsHook, ObservabilityHook, ObservabilityHooks, PatternEvent,
};
pub use spans::{SpanGuard, create_span, record_error, record_success};

// Event categories
pub use hooks::{
    BulkheadEventCategory, CircuitBreakerEventCategory, Event, EventCategory,
    RateLimiterEventCategory, RetryEventCategory, TimeoutEventCategory,
};

// Metrics
pub use hooks::{Metric, metrics};

// Pattern spans
pub use spans::{PatternCategory, PatternSpanGuard};
