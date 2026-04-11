//! Resilience patterns for building fault-tolerant Rust services.
//!
//! Seven patterns — retry, circuit breaker, bulkhead, rate limiter, timeout,
//! hedge, load shed — composable via [`ResiliencePipeline`].
//!
//! Every pattern returns [`CallError<E>`] where `E` is your own error type —
//! no forced mapping, no type erasure.
//!
//! # Quick Start — Pipeline
//!
//! ```rust,no_run
//! use std::time::Duration;
//!
//! use nebula_resilience::{
//!     CallError, ResiliencePipeline,
//!     retry::{BackoffConfig, RetryConfig},
//! };
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let pipeline = ResiliencePipeline::<String>::builder()
//!     .timeout(Duration::from_secs(5))
//!     .retry(RetryConfig::new(3)?.backoff(BackoffConfig::exponential_default()))
//!     .build();
//!
//! let _value: Result<String, _> = pipeline
//!     .call(|| Box::pin(async { Ok::<_, String>("success".into()) }))
//!     .await;
//! # Ok(())
//! # }
//! ```
//!
//! # Standalone Patterns
//!
//! Each pattern also works independently:
//!
//! ```rust,no_run
//! use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
//! use nebula_resilience::retry::{RetryConfig, BackoffConfig, retry_with};
//! use nebula_resilience::CallError;
//! use std::time::Duration;
//!
//! # #[derive(Debug)]
//! # struct MyError;
//! # impl std::fmt::Display for MyError {
//! #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "error") }
//! # }
//! # impl std::error::Error for MyError {}
//! # impl nebula_error::Classify for MyError {
//! #     fn category(&self) -> nebula_error::ErrorCategory { nebula_error::ErrorCategory::Internal }
//! #     fn code(&self) -> nebula_error::ErrorCode { nebula_error::ErrorCode::new("DOC:EXAMPLE") }
//! # }
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Circuit breaker
//! let cb = CircuitBreaker::new(CircuitBreakerConfig {
//!     failure_threshold: 5,
//!     reset_timeout: Duration::from_secs(30),
//!     ..Default::default()
//! })?;
//!
//! let result = cb.call(|| Box::pin(async {
//!     Ok::<_, MyError>("ok")
//! })).await;
//!
//! // Retry with Classify-aware error filtering
//! let config = RetryConfig::<MyError>::new(3)?
//!     .backoff(BackoffConfig::Fixed(Duration::from_millis(50)));
//!
//! let result = retry_with(config, || Box::pin(async {
//!     Ok::<_, MyError>("ok")
//! })).await;
//! # Ok(())
//! # }
//! ```
//!
//! # Error Model
//!
//! [`CallError<E>`] is `#[non_exhaustive]` with variants for each pattern:
//!
//! | Variant | Retryable | Produced by |
//! |---------|-----------|-------------|
//! | `Operation(E)` | depends on `E` | user's operation |
//! | `Timeout(Duration)` | yes | timeout, bulkhead queue |
//! | `RateLimited { retry_after }` | yes | rate limiter |
//! | `BulkheadFull` | yes | bulkhead |
//! | `CircuitOpen` | no | circuit breaker |
//! | `RetriesExhausted { attempts, last }` | no | retry |
//! | `Cancelled { reason }` | no | cancellation |
//! | `LoadShed` | no | load shedder |
//! | `FallbackFailed { reason }` | no | fallback |
//!
//! # Observability
//!
//! Inject a [`MetricsSink`] into any pattern to receive [`ResilienceEvent`]s.
//! Use [`RecordingSink`] in tests for assertion-friendly event capture.

#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::perf)]
// Reason: types like CircuitBreakerConfig deliberately repeat the module name for readability.
#![allow(clippy::module_name_repetitions)]
#![warn(missing_docs)]
#![deny(unsafe_code)]

// ── Modules ────────────────────────────────────────────────────────────────

// Core
pub mod cancellation;
pub mod classifier;
pub mod error;
pub mod policy;

// Observability
pub mod sink;

// Patterns
pub mod bulkhead;
pub mod circuit_breaker;
pub mod fallback;
pub mod hedge;
pub mod load_shed;
pub mod rate_limiter;
pub mod retry;
pub mod timeout;

// Infrastructure
pub mod clock;
pub mod gate;
pub mod pipeline;

// ── Re-exports ─────────────────────────────────────────────────────────────

// Core types
// Patterns
pub use bulkhead::{Bulkhead, BulkheadConfig};
pub use cancellation::{CancellableFuture, CancellationContext, CancellationExt};
// ── Internals exposed for benchmarking ───────────────────────────────────────
#[doc(hidden)]
pub use circuit_breaker::OutcomeWindow;
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
pub use classifier::{
    AlwaysPermanent, AlwaysTransient, ErrorClass, ErrorClassifier, FnClassifier, NebulaClassifier,
};
pub use error::{CallError, CallErrorKind, CallResult, ConfigError};
pub use fallback::{FallbackStrategy, ValueFallback};
// Infrastructure
pub use gate::{Gate, GateClosed, GateGuard};
#[doc(hidden)]
pub use hedge::LatencyTracker;
pub use hedge::{HedgeConfig, HedgeExecutor};
pub use load_shed::load_shed;
pub use pipeline::{LoadShedPredicate, PipelineBuilder, RateLimitCheck, ResiliencePipeline};
pub use policy::{ConstantLoad, LoadSignal, PolicySource};
pub use rate_limiter::{AdaptiveRateLimiter, LeakyBucket, RateLimiter, SlidingWindow, TokenBucket};
#[doc(hidden)]
pub use retry::retry_with_inner;
pub use retry::{BackoffConfig, JitterConfig, RetryConfig, retry, retry_with};
// Observability
pub use sink::{
    CircuitState, MetricsSink, NoopSink, RecordingSink, ResilienceEvent, ResilienceEventKind,
};
pub use timeout::{TimeoutExecutor, timeout};
