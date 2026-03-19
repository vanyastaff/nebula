//! Rate limiting implementations.
//!
//! This module provides multiple rate limiting algorithms:
//!
//! - **`TokenBucket`**: Classic token bucket with refill rate
//! - **`LeakyBucket`**: Leaky bucket with constant leak rate
//! - **`SlidingWindow`**: Sliding time window counter
//! - **`AdaptiveRateLimiter`**: Self-adjusting based on error rates
//! - **`GovernorRateLimiter`**: Production-grade GCRA algorithm
//!
//! # Examples
//!
//! ```
//! use nebula_resilience::patterns::rate_limiter::TokenBucket;
//!
//! let limiter = TokenBucket::new(100, 10.0); // 100 capacity, 10 req/sec
//! ```

use std::future::Future;
use std::sync::Arc;

use crate::ResilienceResult;

// Re-export implementations
mod adaptive;
#[cfg(feature = "governor")]
mod governor_impl;
mod leaky_bucket;
mod sliding_window;
mod token_bucket;

pub use adaptive::AdaptiveRateLimiter;
#[cfg(feature = "governor")]
pub use governor_impl::GovernorRateLimiter;
pub use leaky_bucket::LeakyBucket;
pub use sliding_window::SlidingWindow;
pub use token_bucket::TokenBucket;

/// Rate limiter trait
///
/// Defines the common interface for all rate limiting implementations.
#[allow(async_fn_in_trait)]
pub trait RateLimiter: Send + Sync {
    /// Try to acquire permission for an operation
    ///
    /// Returns `Ok(())` if permission granted, or `Err(RateLimitExceeded)` if rate limit hit.
    async fn acquire(&self) -> ResilienceResult<()>;

    /// Execute an operation with rate limiting
    ///
    /// Acquires permission first, then executes the operation.
    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send;

    /// Get current rate
    ///
    /// Returns the current rate or available capacity (implementation-dependent).
    async fn current_rate(&self) -> f64;

    /// Reset the rate limiter
    ///
    /// Clears all state and resets to initial conditions.
    async fn reset(&self);
}

/// Enum wrapper for dyn-compatible rate limiters
///
/// Allows storing different rate limiter types in a single enum for
/// dynamic dispatch without trait objects.
///
/// The `Governor` variant is only available when the `governor` feature is enabled.
#[derive(Clone)]
pub enum AnyRateLimiter {
    /// Token bucket rate limiter
    TokenBucket(Arc<TokenBucket>),
    /// Leaky bucket rate limiter
    LeakyBucket(Arc<LeakyBucket>),
    /// Sliding window rate limiter
    SlidingWindow(Arc<SlidingWindow>),
    /// Adaptive rate limiter
    Adaptive(Arc<AdaptiveRateLimiter>),
    /// Governor-based GCRA rate limiter (production-grade).
    /// Requires the `governor` feature.
    #[cfg(feature = "governor")]
    Governor(Arc<GovernorRateLimiter>),
}

/// Dispatch a method call to the concrete rate limiter variant.
///
/// Cannot be used with `execute()` because that method has generic type
/// parameters that can't be forwarded through a macro arm.
macro_rules! dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            Self::TokenBucket(l)   => l.$method($($arg),*).await,
            Self::LeakyBucket(l)   => l.$method($($arg),*).await,
            Self::SlidingWindow(l) => l.$method($($arg),*).await,
            Self::Adaptive(l)      => l.$method($($arg),*).await,
            #[cfg(feature = "governor")]
            Self::Governor(l)      => l.$method($($arg),*).await,
        }
    };
}

impl RateLimiter for AnyRateLimiter {
    async fn acquire(&self) -> ResilienceResult<()> {
        dispatch!(self, acquire)
    }

    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        // Generic type parameters prevent use of the dispatch! macro here.
        match self {
            Self::TokenBucket(l)   => l.execute(operation).await,
            Self::LeakyBucket(l)   => l.execute(operation).await,
            Self::SlidingWindow(l) => l.execute(operation).await,
            Self::Adaptive(l)      => l.execute(operation).await,
            #[cfg(feature = "governor")]
            Self::Governor(l)      => l.execute(operation).await,
        }
    }

    async fn current_rate(&self) -> f64 {
        dispatch!(self, current_rate)
    }

    async fn reset(&self) {
        dispatch!(self, reset)
    }
}
