//! Resilience patterns for building fault-tolerant Rust services.
//!
//! Patterns: retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, load shed.
//! All return `CallError<E>` so callers keep their own error type.
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use nebula_resilience::{ResiliencePipeline, CallError};
//! use nebula_resilience::patterns::retry::{RetryConfig, BackoffConfig};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let pipeline = ResiliencePipeline::<&str>::builder()
//!         .timeout(Duration::from_secs(5))
//!         .retry(RetryConfig::new(3)?.backoff(BackoffConfig::Fixed(Duration::from_millis(100))))
//!         .build();
//!
//!     let result = pipeline.call(|| Box::pin(async {
//!         Ok::<_, &str>("success")
//!     })).await;
//!     Ok(())
//! }
//! ```
//!
//! # Circuit Breaker
//!
//! ```rust,no_run
//! use nebula_resilience::patterns::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let cb = CircuitBreaker::new(CircuitBreakerConfig {
//!         failure_threshold: 5,
//!         reset_timeout: Duration::from_secs(30),
//!         ..Default::default()
//!     })?;
//!
//!     let result = cb.call(|| Box::pin(async {
//!         Ok::<_, &str>("success")
//!     })).await;
//!     Ok(())
//! }
//! ```
//!
//! # Retry
//!
//! ```rust,no_run
//! use nebula_resilience::patterns::retry::{RetryConfig, BackoffConfig, retry_with};
//! use nebula_resilience::CallError;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = RetryConfig::<&str>::new(3)?
//!         .backoff(BackoffConfig::Fixed(Duration::from_millis(50)));
//!
//!     let result: Result<u32, CallError<&str>> = retry_with(config, || Box::pin(async {
//!         Ok(42u32)
//!     })).await;
//!     Ok(())
//! }
//! ```

#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::perf)]
// Reason: types like CircuitBreakerConfig deliberately repeat the module name for readability.
#![allow(clippy::module_name_repetitions)]
#![warn(missing_docs)]
#![deny(unsafe_code)]

// Modules
pub mod clock;
pub mod core;
pub mod gate;
pub mod helpers;
pub mod observability;
pub mod patterns;
pub mod pipeline;
pub mod retryable;

// ── Re-exports from core ─────────────────────────────────────────────────────

pub use core::{
    // Error types
    CircuitBreakerOpenState,
    // Policy source
    ConstantLoad,
    ErrorClass,
    ErrorContext,
    LoadSignal,
    // Metrics
    MetricKind,
    MetricSnapshot,
    Metrics,
    MetricsCollector,

    PolicySource,

    ResilienceError,
    ResilienceResult,

    // Core error and result types
    types::{CallError, CallResult, ConfigError},
};

// ── Re-exports from patterns ─────────────────────────────────────────────────

pub use patterns::{
    bulkhead::{Bulkhead, BulkheadConfig},
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, Outcome},
    fallback::{AnyStringFallbackStrategy, FallbackStrategy, ValueFallback},
    hedge::{HedgeConfig, HedgeExecutor},
    load_shed::load_shed,
    rate_limiter::{
        AdaptiveRateLimiter, AnyRateLimiter, LeakyBucket, RateLimiter, SlidingWindow, TokenBucket,
    },
    retry::{BackoffConfig, JitterConfig, RetryConfig, retry, retry_with},
    timeout::{TimeoutExecutor, timeout as timeout_fn, timeout_with_original_error},
};

// ── Other re-exports ─────────────────────────────────────────────────────────

pub use gate::{Gate, GateClosed, GateGuard};
pub use observability::sink::CircuitState;
pub use observability::{MetricsSink, NoopSink, RecordingSink, ResilienceEvent};
pub use pipeline::{PipelineBuilder, ResiliencePipeline};

/// Functional resilience API — convenience functions for simple cases.
pub mod resilience {
    pub use crate::patterns::load_shed::load_shed;
    pub use crate::patterns::retry::{retry, retry_with};
    pub use crate::patterns::timeout::timeout_with_original_error as with_timeout;
}

/// Type-level constants for common configurations.
pub mod constants {
    use std::time::Duration;

    /// Default timeout duration (30 seconds).
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    /// Default number of retry attempts.
    pub const DEFAULT_RETRY_ATTEMPTS: usize = 3;
    /// Default circuit breaker failure threshold.
    pub const DEFAULT_FAILURE_THRESHOLD: usize = 5;
    /// Default rate limit (requests per second).
    pub const DEFAULT_RATE_LIMIT: f64 = 100.0;
}

/// Library version with compile-time embedding
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
