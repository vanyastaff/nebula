//! Advanced rate limiting implementations

use async_trait::async_trait;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use governor::{Quota, RateLimiter as GovernorLimiter, DefaultDirectRateLimiter};
use governor::state::{InMemoryState, NotKeyed};
use governor::clock::{Clock, DefaultClock};

use crate::{ResilienceError, ResilienceResult};

/// Rate limiter trait
#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Try to acquire permission for an operation
    async fn acquire(&self) -> ResilienceResult<()>;

    /// Execute an operation with rate limiting
    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send;

    /// Get current rate
    async fn current_rate(&self) -> f64;

    /// Reset the rate limiter
    async fn reset(&self);
}

/// Enum wrapper for dyn-compatible rate limiters
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

/// Token bucket rate limiter
pub struct TokenBucket {
    /// Maximum tokens in bucket
    capacity: usize,
    /// Tokens currently available
    tokens: Arc<Mutex<f64>>,
    /// Token refill rate per second
    refill_rate: f64,
    /// Last refill timestamp
    last_refill: Arc<Mutex<Instant>>,
    /// Burst size
    burst_size: usize,
}

impl TokenBucket {
    /// Create new token bucket with validation
    #[must_use] 
    pub fn new(capacity: usize, refill_rate: f64) -> Self {
        // Security: prevent creating token buckets with invalid parameters
        let safe_capacity = capacity.min(100_000); // Prevent memory exhaustion
        let safe_refill_rate = refill_rate.clamp(0.001, 10_000.0); // Reasonable limits

        Self {
            capacity: safe_capacity,
            tokens: Arc::new(Mutex::new(safe_capacity as f64)),
            refill_rate: safe_refill_rate,
            last_refill: Arc::new(Mutex::new(Instant::now())),
            burst_size: safe_capacity,
        }
    }

    /// Set burst size
    #[must_use = "builder methods must be chained or built"]
    pub fn with_burst(mut self, burst_size: usize) -> Self {
        self.burst_size = burst_size;
        self
    }

    /// Refill tokens based on elapsed time
    async fn refill(&self) {
        let mut tokens = self.tokens.lock().await;
        let mut last_refill = self.last_refill.lock().await;

        let now = Instant::now();
        let elapsed = now.duration_since(*last_refill).as_secs_f64();

        let tokens_to_add = elapsed * self.refill_rate;
        *tokens = (*tokens + tokens_to_add).min(self.capacity as f64);
        *last_refill = now;
    }
}

#[async_trait]
impl RateLimiter for TokenBucket {
    async fn acquire(&self) -> ResilienceResult<()> {
        self.refill().await;

        let mut tokens = self.tokens.lock().await;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(ResilienceError::RateLimitExceeded {
                retry_after: Some(Duration::from_secs_f64(1.0 / self.refill_rate)),
                limit: self.refill_rate,
                current: self.refill_rate + 1.0, // Over limit
            })
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
        let tokens = self.tokens.lock().await;
        *tokens
    }

    async fn reset(&self) {
        let mut tokens = self.tokens.lock().await;
        *tokens = self.capacity as f64;
    }
}

/// Leaky bucket rate limiter
pub struct LeakyBucket {
    /// Bucket capacity
    capacity: usize,
    /// Current water level
    level: Arc<Mutex<usize>>,
    /// Leak rate per second
    leak_rate: f64,
    /// Last leak timestamp
    last_leak: Arc<Mutex<Instant>>,
    /// RAII guard - Semaphore for blocking/coordination.
    /// Not directly accessed but maintains resource lifecycle.
    #[allow(dead_code)]
    semaphore: Arc<Semaphore>,
}

impl LeakyBucket {
    /// Create new leaky bucket
    #[must_use] 
    pub fn new(capacity: usize, leak_rate: f64) -> Self {
        Self {
            capacity,
            level: Arc::new(Mutex::new(0)),
            leak_rate,
            last_leak: Arc::new(Mutex::new(Instant::now())),
            semaphore: Arc::new(Semaphore::new(capacity)),
        }
    }

    /// Process leaks based on elapsed time
    async fn leak(&self) {
        let mut level = self.level.lock().await;
        let mut last_leak = self.last_leak.lock().await;

        let now = Instant::now();
        let elapsed = now.duration_since(*last_leak).as_secs_f64();

        let leaked = (elapsed * self.leak_rate) as usize;
        *level = level.saturating_sub(leaked);
        *last_leak = now;
    }
}

#[async_trait]
impl RateLimiter for LeakyBucket {
    async fn acquire(&self) -> ResilienceResult<()> {
        self.leak().await;

        let mut level = self.level.lock().await;
        if *level < self.capacity {
            *level += 1;
            Ok(())
        } else {
            Err(ResilienceError::RateLimitExceeded {
                retry_after: Some(Duration::from_secs_f64(1.0 / self.leak_rate)),
                limit: self.capacity as f64,
                current: (self.capacity + 1) as f64, // Over capacity
            })
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
        let level = self.level.lock().await;
        (self.capacity - *level) as f64
    }

    async fn reset(&self) {
        let mut level = self.level.lock().await;
        *level = 0;
    }
}

/// Sliding window rate limiter
pub struct SlidingWindow {
    /// Window duration
    window_duration: Duration,
    /// Maximum requests per window
    max_requests: usize,
    /// Request timestamps
    requests: Arc<Mutex<VecDeque<Instant>>>,
}

impl SlidingWindow {
    /// Create new sliding window rate limiter
    #[must_use] 
    pub fn new(window_duration: Duration, max_requests: usize) -> Self {
        Self {
            window_duration,
            max_requests,
            requests: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Clean old requests outside the window
    async fn clean_old_requests(&self) {
        let mut requests = self.requests.lock().await;
        let cutoff = Instant::now().checked_sub(self.window_duration).unwrap();

        while let Some(&front) = requests.front() {
            if front < cutoff {
                requests.pop_front();
            } else {
                break;
            }
        }
    }
}

#[async_trait]
impl RateLimiter for SlidingWindow {
    async fn acquire(&self) -> ResilienceResult<()> {
        self.clean_old_requests().await;

        let mut requests = self.requests.lock().await;
        if requests.len() < self.max_requests {
            requests.push_back(Instant::now());
            Ok(())
        } else {
            // Calculate retry after based on oldest request
            let retry_after = requests
                .front()
                .map_or(Duration::from_secs(1), |&oldest| self.window_duration.checked_sub(oldest.elapsed()).unwrap());

            Err(ResilienceError::RateLimitExceeded {
                retry_after: Some(retry_after),
                limit: self.max_requests as f64,
                current: (self.max_requests + 1) as f64, // Over limit
            })
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
        self.clean_old_requests().await;
        let requests = self.requests.lock().await;
        requests.len() as f64
    }

    async fn reset(&self) {
        let mut requests = self.requests.lock().await;
        requests.clear();
    }
}

/// Adaptive rate limiter that adjusts based on error rates
pub struct AdaptiveRateLimiter {
    /// Current rate limiter
    inner: Arc<Mutex<AnyRateLimiter>>,
    /// Success count
    success_count: Arc<Mutex<usize>>,
    /// Error count
    error_count: Arc<Mutex<usize>>,
    /// Window for statistics
    stats_window: Duration,
    /// Last stats reset
    last_stats_reset: Arc<Mutex<Instant>>,
    /// Minimum rate
    min_rate: f64,
    /// Maximum rate
    max_rate: f64,
    /// Current rate
    current_rate: Arc<Mutex<f64>>,
}

impl AdaptiveRateLimiter {
    /// Create new adaptive rate limiter
    #[must_use] 
    pub fn new(initial_rate: f64, min_rate: f64, max_rate: f64) -> Self {
        let token_bucket = TokenBucket::new(initial_rate as usize, initial_rate);

        Self {
            inner: Arc::new(Mutex::new(AnyRateLimiter::TokenBucket(Arc::new(
                token_bucket,
            )))),
            success_count: Arc::new(Mutex::new(0)),
            error_count: Arc::new(Mutex::new(0)),
            stats_window: Duration::from_secs(60),
            last_stats_reset: Arc::new(Mutex::new(Instant::now())),
            min_rate,
            max_rate,
            current_rate: Arc::new(Mutex::new(initial_rate)),
        }
    }

    /// Adjust rate based on error rate
    async fn adjust_rate(&self) {
        let mut last_reset = self.last_stats_reset.lock().await;

        if last_reset.elapsed() >= self.stats_window {
            let success = *self.success_count.lock().await;
            let errors = *self.error_count.lock().await;
            let total = success + errors;

            if total > 0 {
                let error_rate = errors as f64 / total as f64;
                let mut current_rate = self.current_rate.lock().await;

                // Adjust rate based on error rate
                if error_rate > 0.1 {
                    // High error rate - decrease rate
                    *current_rate = (*current_rate * 0.9).max(self.min_rate);
                } else if error_rate < 0.01 {
                    // Low error rate - increase rate
                    *current_rate = (*current_rate * 1.1).min(self.max_rate);
                }

                // Update inner rate limiter
                let new_limiter = TokenBucket::new(*current_rate as usize, *current_rate);
                *self.inner.lock().await = AnyRateLimiter::TokenBucket(Arc::new(new_limiter));
            }

            // Reset stats
            *self.success_count.lock().await = 0;
            *self.error_count.lock().await = 0;
            *last_reset = Instant::now();
        }
    }

    /// Record success
    pub async fn record_success(&self) {
        *self.success_count.lock().await += 1;
        self.adjust_rate().await;
    }

    /// Record error
    pub async fn record_error(&self) {
        *self.error_count.lock().await += 1;
        self.adjust_rate().await;
    }
}

#[async_trait]
impl RateLimiter for AdaptiveRateLimiter {
    async fn acquire(&self) -> ResilienceResult<()> {
        self.inner.lock().await.acquire().await
    }

    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        self.acquire().await?;
        let result = operation().await;

        match &result {
            Ok(_) => self.record_success().await,
            Err(_) => self.record_error().await,
        }

        result
    }

    async fn current_rate(&self) -> f64 {
        *self.current_rate.lock().await
    }

    async fn reset(&self) {
        self.inner.lock().await.reset().await;
        *self.success_count.lock().await = 0;
        *self.error_count.lock().await = 0;
    }
}

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
pub struct GovernorRateLimiter {
    /// Inner governor rate limiter
    limiter: DefaultDirectRateLimiter,
    /// Request rate for metrics
    rate_per_second: f64,
    /// Burst capacity
    burst_capacity: u32,
}

impl GovernorRateLimiter {
    /// Create a new governor-based rate limiter
    ///
    /// # Arguments
    /// * `rate_per_second` - Number of requests allowed per second
    /// * `burst_capacity` - Maximum burst size (how many requests can be made instantly)
    ///
    /// # Examples
    /// ```
    /// use nebula_resilience::patterns::rate_limiter::GovernorRateLimiter;
    ///
    /// // Allow 100 requests per second with burst of 10
    /// let limiter = GovernorRateLimiter::new(100.0, 10);
    /// ```
    #[must_use]
    pub fn new(rate_per_second: f64, burst_capacity: u32) -> Self {
        // Security: validate inputs
        let safe_rate = rate_per_second.clamp(0.001, 1_000_000.0);
        let safe_burst = burst_capacity.min(100_000);

        // Convert rate to quota
        // governor uses NonZeroU32 for rate, so we need to be careful
        let rate_u32 = safe_rate.ceil() as u32;
        let quota = Quota::per_second(std::num::NonZeroU32::new(rate_u32.max(1)).unwrap())
            .allow_burst(std::num::NonZeroU32::new(safe_burst.max(1)).unwrap());

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
