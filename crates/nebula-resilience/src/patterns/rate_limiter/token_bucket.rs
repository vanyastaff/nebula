//! Token bucket rate limiter implementation

use async_trait::async_trait;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::RateLimiter;
use crate::{ResilienceError, ResilienceResult};

/// Token bucket rate limiter
///
/// Classic token bucket algorithm with configurable capacity and refill rate.
/// Tokens are added at a constant rate and consumed by operations.
///
/// # Security
/// - Maximum capacity limited to 100,000 to prevent memory exhaustion
/// - Refill rate clamped between 0.001 and 10,000 req/sec
///
/// # Examples
///
/// ```
/// use nebula_resilience::patterns::rate_limiter::TokenBucket;
///
/// let limiter = TokenBucket::new(100, 10.0); // 100 capacity, 10 req/sec
/// ```
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
