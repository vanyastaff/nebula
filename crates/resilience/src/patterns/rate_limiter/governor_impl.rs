//! Governor-based GCRA rate limiter implementation

use governor::clock::Clock;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter as GovernorLimiter};
use std::future::Future;
use std::num::NonZeroU32;
use std::time::Duration;

use super::RateLimiter;
use crate::CallError;

/// Governor-based rate limiter using GCRA (Generic Cell Rate Algorithm)
///
/// Production-grade, sub-millisecond precision, lock-free implementation.
pub struct GovernorRateLimiter {
    limiter: DefaultDirectRateLimiter,
    rate_per_second: f64,
    burst_capacity: u32,
}

impl GovernorRateLimiter {
    /// Create a new governor-based rate limiter.
    #[must_use]
    pub fn new(rate_per_second: f64, burst_capacity: u32) -> Self {
        let safe_rate = rate_per_second.clamp(0.001, 1_000_000.0);
        let safe_burst = burst_capacity.min(100_000);
        let burst = NonZeroU32::new(safe_burst.max(1)).unwrap_or(NonZeroU32::MIN);

        let request_period = Duration::from_secs_f64(1.0 / safe_rate).max(Duration::from_nanos(1));
        let quota = Quota::with_period(request_period).map_or_else(
            || Quota::per_second(NonZeroU32::MIN).allow_burst(burst),
            |base| base.allow_burst(burst),
        );

        Self {
            limiter: GovernorLimiter::direct(quota),
            rate_per_second: safe_rate,
            burst_capacity: safe_burst,
        }
    }

    /// Returns the configured rate per second.
    #[must_use]
    pub const fn rate_per_second(&self) -> f64 {
        self.rate_per_second
    }

    /// Returns the configured burst capacity.
    #[must_use]
    pub const fn burst_capacity(&self) -> u32 {
        self.burst_capacity
    }

    /// Create with custom quota for advanced use cases.
    #[must_use]
    pub fn with_quota(quota: Quota) -> Self {
        Self {
            limiter: GovernorLimiter::direct(quota),
            rate_per_second: 0.0,
            burst_capacity: 0,
        }
    }
}

impl RateLimiter for GovernorRateLimiter {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        match self.limiter.check() {
            Ok(()) => Ok(()),
            Err(_) => Err(CallError::RateLimited),
        }
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
        self.rate_per_second
    }

    async fn reset(&self) {
        // GCRA state decays naturally — no-op for trait compatibility
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limited_after_burst_exhausted() {
        let limiter = GovernorRateLimiter::new(10.0, 5);

        for _ in 0..5 {
            assert!(limiter.acquire().await.is_ok());
        }

        let result = limiter.acquire().await;
        assert!(matches!(result, Err(CallError::RateLimited)));
    }

    #[tokio::test]
    async fn execute_succeeds_within_capacity() {
        let limiter = GovernorRateLimiter::new(100.0, 10);
        let result = limiter.execute(|| async { Ok::<i32, &str>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }
}
