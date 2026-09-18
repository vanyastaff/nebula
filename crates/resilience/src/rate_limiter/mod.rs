//! Rate limiting implementations.
//!
//! [`RateLimiter`] is the algorithm-independent contract; each implementation
//! lives in its own submodule:
//!
//! | Implementation | Algorithm | Best for |
//! |---|---|---|
//! | [`TokenBucket`] | Token bucket with configurable refill rate | Bursty traffic with a steady average |
//! | [`LeakyBucket`] | Leaky bucket with constant drain rate | Smoothing bursts into a constant outflow |
//! | [`SlidingWindow`] | Sliding time-window counter | Hard per-window request caps |
//! | [`AdaptiveRateLimiter`] | Token bucket auto-tuned by error rate | Self-protecting services with variable load |
//!
//! # Standalone usage
//!
//! ```rust
//! use nebula_resilience::{RateLimiter, rate_limiter::TokenBucket};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let limiter = TokenBucket::new(100, 10.0).unwrap();
//! limiter.acquire().await.expect("rate limit not yet reached");
//! let status = limiter.status().await;
//! assert!(status.remaining <= 100.0);
//! # }
//! ```
//!
//! # Pipeline integration
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use nebula_resilience::{ResiliencePipeline, rate_limiter::TokenBucket};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let rl = Arc::new(TokenBucket::new(100, 10.0)?);
//! let pipeline = ResiliencePipeline::<String>::builder()
//!     .rate_limiter_from(rl)
//!     .build();
//! # let _ = pipeline;
//! # Ok(())
//! # }
//! ```

//! Rate limiting implementations.
//!
//! This module provides the [`RateLimiter`] trait and multiple built-in algorithms:
//!
//! | Implementation | Algorithm | Best for |
//! |---|---|---|
//! | [`TokenBucket`] | Token bucket with configurable refill rate | Bursty traffic with a steady average |
//! | [`LeakyBucket`] | Leaky bucket with constant drain rate | Smoothing request bursts into a constant outflow |
//! | [`SlidingWindow`] | Sliding time-window counter | Hard per-window request caps |
//! | [`AdaptiveRateLimiter`] | Token bucket auto-tuned by error rate | Self-protecting services with variable load |
//! # Trait contract
//!
//! Every implementation must satisfy the [`RateLimiter`] contract:
//!
//! - [`acquire()`](RateLimiter::acquire) — attempt to consume one permit. Returns `Ok(())` when the
//!   request is allowed, `Err(CallError::RateLimited)` when the limit is exceeded.
//! - [`call()`](RateLimiter::call) — convenience wrapper that calls `acquire()` then executes the
//!   supplied async closure. On success the closure's return value is forwarded; on rate-limit the
//!   closure is never invoked.
//! - Implementors must be `Send + Sync` so they can be shared across tasks and stored inside
//!   `Arc<T>`.
//!
//! # Standalone usage
//!
//! All rate limiters work independently — no pipeline required:
//!
//! ```rust
//! use nebula_resilience::{RateLimiter, rate_limiter::TokenBucket};
//!
//! # #[tokio::main]
//! # async fn main() {
//! // Allow up to 100 tokens; refill at 10 tokens/second.
//! let limiter = TokenBucket::new(100, 10.0).unwrap();
//!
//! // Returns Ok(()) when a permit is available.
//! limiter.acquire().await.expect("rate limit not yet reached");
//!
//! // Or use call() to gate an operation:
//! let result = limiter
//!     .call(|| async { Ok::<&str, &str>("response") })
//!     .await;
//! assert!(result.is_ok());
//! # }
//! ```
//!
//! # Pipeline integration
//!
//! When using a rate limiter inside a [`ResiliencePipeline`](crate::ResiliencePipeline) the
//! limiter must be wrapped in [`Arc`] before being passed to
//! [`rate_limiter_from()`](crate::PipelineBuilder::rate_limiter_from):
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use nebula_resilience::{ResiliencePipeline, rate_limiter::TokenBucket};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Arc is required so the limiter can be cloned into the async closure
//! // that the pipeline builds internally — the closure must be
//! // `Send + Sync + 'static`, which demands shared ownership.
//! let rl = Arc::new(TokenBucket::new(100, 10.0)?);
//!
//! let pipeline = ResiliencePipeline::<String>::builder()
//!     .rate_limiter_from(rl)
//!     .build();
//!
//! let _result: Result<String, _> = pipeline
//!     .call(|| Box::pin(async { Ok::<_, String>("ok".into()) }))
//!     .await;
//! # Ok(())
//! # }
//! ```

use std::{future::Future, pin::Pin, time::Duration};

use crate::{CallContext, CallError};

fn retry_after_from_rate(units_needed: f64, units_per_second: f64) -> Option<Duration> {
    if !units_needed.is_finite() || !units_per_second.is_finite() || units_per_second <= 0.0 {
        return None;
    }
    if units_needed <= 0.0 {
        return Some(Duration::ZERO);
    }

    let seconds = units_needed / units_per_second;
    if !seconds.is_finite() {
        return None;
    }

    Some(Duration::from_secs_f64(seconds).max(Duration::from_nanos(1)))
}

fn duration_as_nanos_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn rate_limited_with_retry_after(retry_after: Option<Duration>) -> CallError<()> {
    retry_after.map_or_else(CallError::rate_limited, CallError::rate_limited_after)
}

pub(crate) fn map_acquire_error<E>(err: CallError<()>) -> CallError<E> {
    match err {
        CallError::RateLimited { retry_after } => CallError::RateLimited { retry_after },
        CallError::Timeout(duration) => CallError::Timeout(duration),
        CallError::CircuitOpen => CallError::CircuitOpen,
        CallError::BulkheadFull => CallError::BulkheadFull,
        CallError::Cancelled { reason } => CallError::Cancelled { reason },
        CallError::TaskPanicked => CallError::TaskPanicked,
        CallError::LoadShed => CallError::LoadShed,
        CallError::FallbackFailed { reason } => CallError::FallbackFailed { reason },
        CallError::FallbackFailedWithContext { primary, fallback } => {
            CallError::FallbackFailedWithContext {
                primary: Box::new(map_acquire_error(*primary)),
                fallback: Box::new(map_acquire_error(*fallback)),
            }
        },
        CallError::Operation(()) | CallError::RetriesExhausted { .. } => CallError::rate_limited(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// STATUS
// ═══════════════════════════════════════════════════════════════════════════════

/// Current state of a rate limiter, uniform across algorithms.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::{RateLimiter, rate_limiter::TokenBucket};
///
/// # #[tokio::main]
/// # async fn main() {
/// let limiter = TokenBucket::new(10, 1.0).expect("valid config");
/// let status = limiter.status().await;
/// assert!(status.remaining <= 10.0);
/// assert_eq!(status.limit_per_second, Some(1.0));
/// # }
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct RateLimiterStatus {
    /// Permits available right now, in the limiter's own permit units.
    ///
    /// This is the number of immediately successful `acquire()` calls
    /// (fractional when the bucket refills continuously). It is never a
    /// rate: `SlidingWindow` reports its remaining window quota here.
    pub remaining: f64,
    /// Configured steady-state rate in permits per second, when the
    /// algorithm has one.
    ///
    /// `TokenBucket` and `LeakyBucket` fill this from their refill/leak rate;
    /// `AdaptiveRateLimiter` from its current tuned rate. `SlidingWindow`
    /// returns `None` — it enforces a count per window, not a rate.
    pub limit_per_second: Option<f64>,
}

impl RateLimiterStatus {
    /// Build a status from a remaining-quota count and an optional rate.
    #[must_use]
    pub const fn new(remaining: f64, limit_per_second: Option<f64>) -> Self {
        Self {
            remaining,
            limit_per_second,
        }
    }
}

/// Core abstraction for all rate-limiting implementations.
///
/// # Contract
///
/// - [`acquire()`](RateLimiter::acquire) **must** return `Ok(())` when a permit is granted and
///   <code>Err([`CallError::RateLimited`])</code> when the rate limit is exceeded. It must never
///   block indefinitely — implementations that queue callers should enforce a timeout or queue
///   bound.
/// - [`call()`](RateLimiter::call) is a convenience wrapper over `acquire()` + operation
///   invocation. The default implementation is correct for the vast majority of cases. Override it
///   only when you need to observe the operation result (e.g., [`AdaptiveRateLimiter`] tracks
///   errors to tune its rate).
/// - **Thread safety**: all implementors must be `Send + Sync` so they can be shared across async
///   tasks via `Arc<T>`.
///
/// # Why `acquire` is `async`
///
/// A limiter that enforces a strict outflow rate must be able to park a caller
/// until a permit is free; a synchronous `Result` signature would force such
/// limiters to spin or lie. The built-in limiters all fail fast instead (they
/// return `RateLimited` immediately), so their `async` bodies complete without
/// awaiting — that is an implementation choice, not the contract. Implementors
/// that queue callers must bound the wait themselves.
///
/// Because `impl Future` in return position makes the trait not object-safe,
/// [`ErasedRateLimiter`] is the object-safe facade for heterogeneous registries.
///
/// # Implementing for third-party types
///
/// This trait is `sealed`-free and designed for downstream implementation.
/// New methods will always provide default implementations to avoid breaking
/// changes across minor versions.
///
/// [`TokenBucket`] and [`SlidingWindow`] are complete implementations to copy
/// from; the [module documentation](self) shows both in use.
pub trait RateLimiter: Send + Sync {
    /// Attempt to consume one permit from the rate limiter.
    ///
    /// Returns `Ok(())` when the request is allowed to proceed, or
    /// <code>Err([`CallError::RateLimited`])</code> when the current rate would be
    /// exceeded. The error may include a `retry_after` hint for callers
    /// that want to back off before retrying.
    fn acquire(&self) -> impl Future<Output = Result<(), CallError<()>>> + Send;

    /// Attempt to consume one permit with cancellation/deadline from a shared
    /// policy context.
    ///
    /// Implementations do not need to override this unless they can enforce the
    /// context more efficiently internally.
    fn acquire_with_context<'a>(
        &'a self,
        context: &'a CallContext,
    ) -> impl Future<Output = Result<(), CallError<()>>> + Send + 'a {
        async move { context.run_result(self.acquire()).await }
    }

    /// Acquire a permit and, if successful, execute `operation`.
    ///
    /// Returns <code>Err([`CallError::RateLimited`])</code> without calling `operation`
    /// when the rate limit is exceeded, or the operation's own error wrapped in
    /// [`CallError::Operation`] when `operation`
    /// itself fails.
    ///
    /// The default implementation calls [`acquire()`](Self::acquire) then
    /// invokes `operation`. Override this only when you need to observe the
    /// operation result — for example, to adjust internal counters as
    /// [`AdaptiveRateLimiter`] does.
    fn call<T, E, F, Fut>(
        &self,
        operation: F,
    ) -> impl Future<Output = Result<T, CallError<E>>> + Send
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send,
    {
        async {
            self.acquire().await.map_err(map_acquire_error)?;
            operation().await.map_err(CallError::Operation)
        }
    }

    /// Acquire a permit using `context`, then execute `operation` under the
    /// same cancellation/deadline contract.
    ///
    /// The operation is not invoked when the limiter rejects, the context is
    /// cancelled, or the context deadline expires before a permit is acquired.
    fn call_with_context<'a, T, E, F, Fut>(
        &'a self,
        context: &'a CallContext,
        operation: F,
    ) -> impl Future<Output = Result<T, CallError<E>>> + Send + 'a
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
        T: Send + 'a,
        E: Send + 'a,
    {
        async move {
            self.acquire_with_context(context)
                .await
                .map_err(map_acquire_error)?;
            context
                .run_result(async { operation().await.map_err(CallError::Operation) })
                .await
        }
    }

    /// Report the limiter's current state.
    ///
    /// [`remaining`](RateLimiterStatus::remaining) answers "would `acquire()`
    /// succeed right now, and how many times?" for every implementation.
    /// [`limit_per_second`](RateLimiterStatus::limit_per_second) is the
    /// configured steady-state rate where the algorithm has one
    /// (`None` for window counters, whose whole point is the window, not a
    /// rate).
    ///
    /// The field names follow the IETF `RateLimit` header vocabulary
    /// (`RateLimit-Remaining` / `RateLimit-Limit`): a status reports quota,
    /// not an algorithm-specific internal counter.
    fn status(&self) -> impl Future<Output = RateLimiterStatus> + Send;

    /// Clears all state and resets to initial conditions.
    fn reset(&self) -> impl Future<Output = ()> + Send;
}

type BoxRateLimiterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe facade for storing heterogeneous rate limiters in registries.
///
/// [`RateLimiter`] keeps `impl Future` methods for static dispatch and zero
/// allocation in direct calls, which makes that trait intentionally not
/// object-safe. Use `Arc<dyn ErasedRateLimiter>` when a tenant/resource registry
/// needs to store multiple limiter implementations behind one type.
///
/// **No registry in this workspace stores `dyn ErasedRateLimiter` yet.** It is
/// the seam a host uses when limiter selection is dynamic (tenant- or
/// resource-scoped policies); the engine currently builds one concrete
/// `TokenBucket` per action key and does not need the facade. It is kept
/// because removing object safety from `RateLimiter` later would be a breaking
/// change, not because there is a current consumer.
///
/// The facade exposes only non-generic operations. To run an operation through a
/// limiter, acquire through this trait and then call the operation yourself, or
/// pass the object to
/// [`PipelineBuilder::rate_limiter_erased`](crate::PipelineBuilder::rate_limiter_erased).
///
/// The [module documentation](self) shows a `Vec<Arc<dyn ErasedRateLimiter>>`
/// registry.
pub trait ErasedRateLimiter: Send + Sync {
    /// Attempt to consume one permit from the rate limiter.
    fn acquire_boxed(&self) -> BoxRateLimiterFuture<'_, Result<(), CallError<()>>>;

    /// Attempt to consume one permit with cancellation/deadline from a shared
    /// policy context.
    fn acquire_with_context_boxed<'a>(
        &'a self,
        context: &'a CallContext,
    ) -> BoxRateLimiterFuture<'a, Result<(), CallError<()>>> {
        Box::pin(context.run_result(self.acquire_boxed()))
    }

    /// Report the limiter's current state (see [`RateLimiter::status`]).
    fn status_boxed(&self) -> BoxRateLimiterFuture<'_, RateLimiterStatus>;

    /// Clears all state and resets to initial conditions.
    fn reset_boxed(&self) -> BoxRateLimiterFuture<'_, ()>;
}

impl<T> ErasedRateLimiter for T
where
    T: RateLimiter,
{
    fn acquire_boxed(&self) -> BoxRateLimiterFuture<'_, Result<(), CallError<()>>> {
        Box::pin(self.acquire())
    }

    fn acquire_with_context_boxed<'a>(
        &'a self,
        context: &'a CallContext,
    ) -> BoxRateLimiterFuture<'a, Result<(), CallError<()>>> {
        Box::pin(self.acquire_with_context(context))
    }

    fn status_boxed(&self) -> BoxRateLimiterFuture<'_, RateLimiterStatus> {
        Box::pin(self.status())
    }

    fn reset_boxed(&self) -> BoxRateLimiterFuture<'_, ()> {
        Box::pin(self.reset())
    }
}

pub mod adaptive;
pub mod leaky_bucket;
pub mod sliding_window;
pub mod token_bucket;

pub use adaptive::AdaptiveRateLimiter;
pub use leaky_bucket::LeakyBucket;
pub use sliding_window::SlidingWindow;
pub use token_bucket::TokenBucket;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
