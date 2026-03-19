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

use crate::{
    CallError,
    observability::sink::{MetricsSink, NoopSink, ResilienceEvent},
};

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

/// Rate limiter trait.
///
/// Returns `Err(CallError::RateLimited)` when the rate limit is exceeded.
#[allow(async_fn_in_trait)]
pub trait RateLimiter: Send + Sync {
    /// Try to acquire permission. Returns `Err(CallError::RateLimited)` if limit hit.
    async fn acquire(&self) -> Result<(), CallError<()>>;

    /// Acquire permission then execute `operation`. Returns `Err(CallError::RateLimited)` or
    /// the operation's own error wrapped in `CallError::Operation`.
    async fn execute<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send;

    /// Returns the current rate or available capacity (implementation-dependent).
    async fn current_rate(&self) -> f64;

    /// Clears all state and resets to initial conditions.
    async fn reset(&self);
}

/// Enum wrapper for dyn-compatible rate limiters.
///
/// The `Governor` variant is only available when the `governor` feature is enabled.
#[derive(Clone)]
pub enum AnyRateLimiterInner {
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

/// Rate limiter with an injectable [`MetricsSink`] for observability.
#[derive(Clone)]
pub struct AnyRateLimiter {
    inner: AnyRateLimiterInner,
    sink: Arc<dyn MetricsSink>,
}

impl AnyRateLimiter {
    /// Wrap a `TokenBucket`.
    pub fn token_bucket(l: TokenBucket) -> Self {
        Self {
            inner: AnyRateLimiterInner::TokenBucket(Arc::new(l)),
            sink: Arc::new(NoopSink),
        }
    }
    /// Wrap a `LeakyBucket`.
    pub fn leaky_bucket(l: LeakyBucket) -> Self {
        Self {
            inner: AnyRateLimiterInner::LeakyBucket(Arc::new(l)),
            sink: Arc::new(NoopSink),
        }
    }
    /// Wrap a `SlidingWindow`.
    pub fn sliding_window(l: SlidingWindow) -> Self {
        Self {
            inner: AnyRateLimiterInner::SlidingWindow(Arc::new(l)),
            sink: Arc::new(NoopSink),
        }
    }
    /// Wrap an `AdaptiveRateLimiter`.
    pub fn adaptive(l: AdaptiveRateLimiter) -> Self {
        Self {
            inner: AnyRateLimiterInner::Adaptive(Arc::new(l)),
            sink: Arc::new(NoopSink),
        }
    }
    #[cfg(feature = "governor")]
    /// Wrap a `GovernorRateLimiter`.
    pub fn governor(l: GovernorRateLimiter) -> Self {
        Self {
            inner: AnyRateLimiterInner::Governor(Arc::new(l)),
            sink: Arc::new(NoopSink),
        }
    }

    /// Inject a metrics sink.
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }
}

macro_rules! dispatch_inner {
    ($inner:expr, $method:ident $(, $arg:expr)*) => {
        match $inner {
            AnyRateLimiterInner::TokenBucket(l)   => l.$method($($arg),*).await,
            AnyRateLimiterInner::LeakyBucket(l)   => l.$method($($arg),*).await,
            AnyRateLimiterInner::SlidingWindow(l) => l.$method($($arg),*).await,
            AnyRateLimiterInner::Adaptive(l)      => l.$method($($arg),*).await,
            #[cfg(feature = "governor")]
            AnyRateLimiterInner::Governor(l)      => l.$method($($arg),*).await,
        }
    };
}

impl RateLimiter for AnyRateLimiter {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let result = dispatch_inner!(&self.inner, acquire);
        if result.is_err() {
            self.sink.record(ResilienceEvent::RateLimitExceeded);
        }
        result
    }

    async fn execute<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send,
    {
        self.acquire().await.map_err(|_| CallError::RateLimited)?;
        operation().await.map_err(CallError::Operation)
    }

    async fn current_rate(&self) -> f64 {
        dispatch_inner!(&self.inner, current_rate)
    }

    async fn reset(&self) {
        dispatch_inner!(&self.inner, reset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RecordingSink;

    #[tokio::test]
    async fn emits_rate_limit_exceeded_event() {
        let sink = RecordingSink::new();
        let limiter =
            AnyRateLimiter::token_bucket(TokenBucket::new(1, 0.001)).with_sink(sink.clone());

        assert!(limiter.acquire().await.is_ok()); // first — succeeds (1 token)
        let _ = limiter.acquire().await; // second — rate limited

        assert!(sink.count("rate_limit_exceeded") > 0);
    }
}
