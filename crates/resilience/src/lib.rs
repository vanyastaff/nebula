//! # nebula-resilience
//!
//! In-process stability patterns for fault-tolerant outbound calls inside Nebula actions.
//!
//! ## What this crate gives you
//!
//! Actions that call external APIs face flaky networks, rate limits, and transient
//! failures. This crate provides the canonical in-process resilience layer: seven
//! patterns — retry, circuit breaker, bulkhead, rate limiter, timeout, hedge, and load
//! shed — plus a fallback mechanism for graceful degradation. Retry filtering is driven
//! by [`nebula_error::Classify`], so "transient vs permanent" is an explicit decision,
//! never folklore in an action body.
//!
//! Every pattern returns [`CallError<E>`], where `E` is your own error type — no forced
//! mapping, no `Box<dyn Error>` erasure.
//!
//! ## Entry points
//!
//! - **Compose several patterns** — [`ResiliencePipeline`], built through
//!   [`PipelineBuilder`]:
//!
//!   ```rust,no_run
//!   use std::time::Duration;
//!
//!   use nebula_resilience::{
//!       ResiliencePipeline,
//!       retry::{BackoffConfig, RetryConfig},
//!   };
//!
//!   # #[tokio::main]
//!   # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!   let pipeline = ResiliencePipeline::<String>::builder()
//!       .timeout(Duration::from_secs(5))
//!       .retry(RetryConfig::new(3)?.backoff(BackoffConfig::exponential_default()))
//!       .build();
//!
//!   let _value: Result<String, _> = pipeline
//!       .call(|| Box::pin(async { Ok::<_, String>("success".into()) }))
//!       .await;
//!   # Ok(())
//!   # }
//!   ```
//!
//! - **Use one pattern standalone** — each module has its own entry point and works
//!   without the pipeline: [`retry::retry_with`], [`CircuitBreaker::call`],
//!   [`Bulkhead::call`], [`timeout()`], [`load_shed()`], [`HedgeExecutor::call`],
//!   [`RateLimiter::acquire`], [`FallbackExecutor::call`].
//!
//!   ```rust,no_run
//!   use std::time::Duration;
//!
//!   use nebula_resilience::{
//!       CallError,
//!       circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
//!       retry::{BackoffConfig, RetryConfig, retry_with},
//!   };
//!
//!   # #[derive(Debug)]
//!   # struct MyError;
//!   # impl std::fmt::Display for MyError {
//!   #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "error") }
//!   # }
//!   # impl std::error::Error for MyError {}
//!   # impl nebula_error::Classify for MyError {
//!   #     fn category(&self) -> nebula_error::ErrorCategory { nebula_error::ErrorCategory::Internal }
//!   #     fn code(&self) -> nebula_error::ErrorCode { nebula_error::ErrorCode::new("DOC:EXAMPLE") }
//!   # }
//!   # #[tokio::main]
//!   # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!   let cb = CircuitBreaker::new(CircuitBreakerConfig {
//!       failure_threshold: 5,
//!       reset_timeout: Duration::from_secs(30),
//!       ..Default::default()
//!   })?;
//!   let _: Result<&str, CallError<MyError>> = cb.call(|| Box::pin(async { Ok("ok") })).await;
//!
//!   let config = RetryConfig::<MyError>::new(3)?
//!       .backoff(BackoffConfig::Fixed(Duration::from_millis(50)));
//!   let _: Result<&str, CallError<MyError>> =
//!       retry_with(config, || Box::pin(async { Ok("ok") })).await;
//!   # Ok(())
//!   # }
//!   ```
//!
//! - **Thread one execution contract through everything** — [`CallContext`] carries the
//!   cancellation token, deadline, and observability scope of a single call, and every
//!   context-aware entry point (`call_with_context`, `timeout_with_context`, …) consumes
//!   it.
//!
//! ## Retry has two layers (canon §11.2)
//!
//! `nebula-resilience` owns **in-action outbound-call retry** — the retry loop around a
//! single external call inside one action attempt. The engine separately owns
//! **operator-declared node retry** (`nebula_workflow::RetryConfig` with persisted
//! attempt accounting).
//! They are disjoint: this crate never re-executes a node, and the engine never
//! second-guesses a pipeline retry. A retryable classification is not permission to
//! repeat an ambiguous remote effect (canon §11.3) — see the [`hedge`] module docs for
//! why speculative duplication must never wrap an effecting call.
//!
//! # Error Model
//!
//! [`CallError<E>`] is `#[non_exhaustive]` with one variant per rejection kind. Use
//! [`CallError::operation`] to reach the caller's own error; use
//! [`CallError::is_retryable`] only as a hint (the retry loop's real decision comes from
//! `E`'s `Classify` implementation).
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
//! | `TaskPanicked` | no | hedge (a spawned attempt panicked) |
//! | `FallbackFailed { .. }` / `FallbackFailedWithContext { .. }` | no | fallback |
//!
//! # Observability
//!
//! Inject an [`EventSink`] into any pattern to receive [`ResilienceEvent`]s; the default
//! is the zero-cost [`NoopSink`], and [`RecordingSink`] captures events for tests.
//! [`EventScope`] tags high-level pipeline events with low-cardinality identifiers.
//!
//! # Cargo features
//!
//! | Feature | Default | Effect |
//! |---------|---------|--------|
//! | `serde` | yes | `Serialize`/`Deserialize` for config and event boundary types. |
//! | `bench-internals` | no | Exposes internal helpers the criterion benches measure; visibility only. |
//!
//! The async surface is tokio-based (`tokio::time`, `tokio-util` cancellation); there is
//! no runtime-agnostic mode.
//!
//! # Where to look next
//!
//! - [`pipeline`] — composing patterns and their recommended order
//! - [`retry`](mod@retry) — backoff, jitter, and `Classify`-aware retry
//! - [`circuit_breaker`] — the state machine and its one accounting model
//! - [`CallContext`] / [`Deadline`] — the execution contract of a call
//! - [`events`] — observability hooks

#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::perf)]
// Reason: types like CircuitBreakerConfig deliberately repeat the module name for readability.
#![allow(clippy::module_name_repetitions)]
#![warn(missing_docs)]
#![deny(unsafe_code)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

// ── Modules ────────────────────────────────────────────────────────────────

// Core
pub mod cancellation;
pub mod classifier;
pub mod context;
pub mod deadline;
pub mod error;
pub mod policy;

// Observability
pub mod events;

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
pub use cancellation::{CancellationContext, CancellationExt};
pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use classifier::{
    AlwaysPermanent, AlwaysTransient, ErrorClass, ErrorClassifier, FnClassifier, NebulaClassifier,
};
pub use context::CallContext;
pub use deadline::Deadline;
pub use error::{CallError, CallErrorKind, CallResult, ConfigError};
pub use fallback::{FallbackExecutor, FallbackStrategy, ValueFallback};
// Infrastructure
pub use gate::{Gate, GateCloseTimeout, GateClosed, GateGuard};
#[cfg(feature = "bench-internals")]
pub use hedge::LatencyTracker;
pub use hedge::{AdaptiveHedgeExecutor, HedgeConfig, HedgeExecutor, HedgeSafety};
pub use load_shed::{
    load_shed, load_shed_with_context, load_shed_with_context_and_sink, load_shed_with_sink,
};
pub use pipeline::{
    LoadShedPredicate, PipelineBuilder, PipelineOutcome, RateLimitCheck, ResiliencePipeline,
};
pub use policy::{ConstantLoad, LoadSignal, LoadSnapshot, PolicySource};
pub use rate_limiter::{
    AdaptiveRateLimiter, ErasedRateLimiter, Gcra, LeakyBucket, RateLimiter, RateLimiterStatus,
    SlidingWindow, TokenBucket,
};
#[cfg(feature = "bench-internals")]
pub use retry::retry_with_inner;
pub use retry::{BackoffConfig, JitterConfig, RetryConfig, retry, retry_with};
// Observability
pub use events::{
    EventScope, EventSink, NoopSink, RecordingSink, ResilienceEvent, ResilienceEventKind,
    ScopeValue,
};
pub use timeout::{TimeoutExecutor, timeout, timeout_with_context, timeout_with_context_and_sink};
