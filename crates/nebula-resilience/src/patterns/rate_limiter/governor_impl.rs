//! Governor-based GCRA rate limiter implementation

use async_trait::async_trait;
use std::future::Future;
use governor::{Quota, RateLimiter as GovernorLimiter, DefaultDirectRateLimiter};
use governor::clock::{Clock, DefaultClock};

use crate::{ResilienceError, ResilienceResult};
use super::RateLimiter;

/// Governor-based rate limiter using GCRA (Generic Cell Rate Algorithm)
///
/// This is a production-grade rate limiter that uses the industry-standard
/// GCRA algorithm, which is more accurate and efficient than simple token bucket.
///
/// Features:
/// - Sub-millisecond precision
/// - No background tasks needed
/// - Lock-free implementation
/// - Handles burst traffic elegantly
///
/// # Examples
///
/// ```
/// use nebula_resilience::patterns::rate_limiter::GovernorRateLimiter;
///
/// let limiter = GovernorRateLimiter::new(100.0, 10); // 100 req/sec, burst 10
/// ```
pub struct GovernorRateLimiter {
    /// Inner governor rate limiter
    limiter: DefaultDirectRateLimiter,
    /// Request rate for metrics
    rate_per_second: f64,
    /// Burst capacity (stored for future metrics/introspection)
    #[allow(dead_code)]
    burst_capacity: u32,
}

impl GovernorRateLimiter {
    /// Create a new governor-based rate limiter
    ///
    /// # Arguments
    /// * `rate_per_second` - Number of requests allowed per second
    /// * `burst_capacity` - Maximum burst size (how many requests can be made instantly)
    #[must_use]
    pub fn new(rate_per_second: f64, burst_capacity: u32) -> Self {
        // Security: validate inputs
        let safe_rate = rate_per_second.clamp(0.001, 1_000_000.0);
        let safe_burst = burst_capacity.min(100_000);

        // Convert rate to quota
        // governor uses NonZeroU32 for rate, so we need to be careful
        let rate_u32 = safe_rate.ceil() as u32;
        let quota = Quota::per_second(std::num::NonZeroU32::new(rate_u32.max(1)).expect("Rate must be > 0"))
            .allow_burst(std::num::NonZeroU32::new(safe_burst.max(1)).expect("Burst must be > 0"));

        Self {
            limiter: GovernorLimiter::direct(quota),
            rate_per_second: safe_rate,
            burst_capacity: safe_burst,
        }
    }

    /// Create with custom quota for advanced use cases
    #[must_use]
    pub fn with_quota(quota: Quota) -> Self {
        Self {
            limiter: GovernorLimiter::direct(quota),
            rate_per_second: 0.0, // Unknown
            burst_capacity: 0,    // Unknown
        }
    }
}

#[async_trait]
impl RateLimiter for GovernorRateLimiter {
    async fn acquire(&self) -> ResilienceResult<()> {
        match self.limiter.check() {
            Ok(_) => Ok(()),
            Err(negative) => {
                // Calculate retry_after from the negative decision
                let wait_duration = negative.wait_time_from(DefaultClock::default().now());

                Err(ResilienceError::RateLimitExceeded {
                    retry_after: Some(wait_duration),
                    limit: self.rate_per_second,
                    current: self.rate_per_second + 1.0, // Over limit
                })
            }
        }
    }

    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        self.acquire().await?;
        operation().await
    }

    async fn current_rate(&self) -> f64 {
        // Governor doesn't expose current rate directly
        // Return configured rate
        self.rate_per_second
    }

    async fn reset(&self) {
        // Governor's GCRA algorithm doesn't need explicit reset
        // State decays naturally over time
        // This is a no-op for compatibility with the trait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_governor_rate_limiter_basic() {
        let limiter = GovernorRateLimiter::new(10.0, 5);

        // Should succeed for burst capacity
        for _ in 0..5 {
            assert!(limiter.acquire().await.is_ok());
        }

        // Should fail after burst exhausted
        let result = limiter.acquire().await;
        assert!(result.is_err());

        if let Err(ResilienceError::RateLimitExceeded { retry_after, .. }) = result {
            assert!(retry_after.is_some());
        } else {
            panic!("Expected RateLimitExceeded error");
        }
    }

    #[tokio::test]
    async fn test_governor_rate_limiter_execute() {
        let limiter = GovernorRateLimiter::new(100.0, 10);

        let result = limiter
            .execute(|| async { Ok::<i32, ResilienceError>(42) })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}
