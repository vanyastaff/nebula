//! Advanced rate limiting implementations
//!
//! This module provides multiple rate limiting algorithms for different use cases:
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

use async_trait::async_trait;
use std::future::Future;
use std::sync::Arc;

use crate::ResilienceResult;

// Re-export implementations
mod adaptive;
mod governor_impl;
mod leaky_bucket;
mod sliding_window;
mod token_bucket;

pub use adaptive::AdaptiveRateLimiter;
pub use governor_impl::GovernorRateLimiter;
pub use leaky_bucket::LeakyBucket;
pub use sliding_window::SlidingWindow;
pub use token_bucket::TokenBucket;

/// Rate limiter trait
///
/// Defines the common interface for all rate limiting implementations.
#[async_trait]
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
    /// Governor-based GCRA rate limiter (production-grade)
    Governor(Arc<GovernorRateLimiter>),
}

#[async_trait]
impl RateLimiter for AnyRateLimiter {
    async fn acquire(&self) -> ResilienceResult<()> {
        match self {
            Self::TokenBucket(limiter) => limiter.acquire().await,
            Self::LeakyBucket(limiter) => limiter.acquire().await,
            Self::SlidingWindow(limiter) => limiter.acquire().await,
            Self::Adaptive(limiter) => limiter.acquire().await,
            Self::Governor(limiter) => limiter.acquire().await,
        }
    }

    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        match self {
            Self::TokenBucket(limiter) => limiter.execute(operation).await,
            Self::LeakyBucket(limiter) => limiter.execute(operation).await,
            Self::SlidingWindow(limiter) => limiter.execute(operation).await,
            Self::Adaptive(limiter) => limiter.execute(operation).await,
            Self::Governor(limiter) => limiter.execute(operation).await,
        }
    }

    async fn current_rate(&self) -> f64 {
        match self {
            Self::TokenBucket(limiter) => limiter.current_rate().await,
            Self::LeakyBucket(limiter) => limiter.current_rate().await,
            Self::SlidingWindow(limiter) => limiter.current_rate().await,
            Self::Adaptive(limiter) => limiter.current_rate().await,
            Self::Governor(limiter) => limiter.current_rate().await,
        }
    }

    async fn reset(&self) {
        match self {
            Self::TokenBucket(limiter) => limiter.reset().await,
            Self::LeakyBucket(limiter) => limiter.reset().await,
            Self::SlidingWindow(limiter) => limiter.reset().await,
            Self::Adaptive(limiter) => limiter.reset().await,
            Self::Governor(limiter) => limiter.reset().await,
        }
    }
}
