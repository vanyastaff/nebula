//! Adaptive rate limiter that adjusts based on error rates

use async_trait::async_trait;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::ResilienceResult;
use super::{RateLimiter, AnyRateLimiter, TokenBucket};

/// Adaptive rate limiter that adjusts based on error rates
///
/// Automatically adjusts rate limiting based on success/error ratios.
/// - High error rate (>10%) → decrease rate
/// - Low error rate (<1%) → increase rate
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
