//! Adaptive rate limiter that adjusts based on error rates

use async_trait::async_trait;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::{AnyRateLimiter, RateLimiter, TokenBucket};
use crate::ResilienceResult;

/// Mutable state behind a single lock.
struct AdaptiveState {
    inner: AnyRateLimiter,
    success_count: usize,
    error_count: usize,
    last_stats_reset: Instant,
    current_rate: f64,
}

/// Adaptive rate limiter that adjusts based on error rates
///
/// Automatically adjusts rate limiting based on success/error ratios.
/// - High error rate (>10%) → decrease rate
/// - Low error rate (<1%) → increase rate
pub struct AdaptiveRateLimiter {
    state: Arc<Mutex<AdaptiveState>>,
    stats_window: Duration,
    min_rate: f64,
    max_rate: f64,
}

impl AdaptiveRateLimiter {
    /// Create new adaptive rate limiter
    #[must_use]
    pub fn new(initial_rate: f64, min_rate: f64, max_rate: f64) -> Self {
        let token_bucket = TokenBucket::new(initial_rate as usize, initial_rate);

        Self {
            state: Arc::new(Mutex::new(AdaptiveState {
                inner: AnyRateLimiter::TokenBucket(Arc::new(token_bucket)),
                success_count: 0,
                error_count: 0,
                last_stats_reset: Instant::now(),
                current_rate: initial_rate,
            })),
            stats_window: Duration::from_secs(60),
            min_rate,
            max_rate,
        }
    }

    /// Adjust rate based on error rate. Caller must hold the lock.
    fn adjust_rate_locked(&self, state: &mut AdaptiveState) {
        if state.last_stats_reset.elapsed() < self.stats_window {
            return;
        }

        let total = state.success_count + state.error_count;
        if total > 0 {
            let error_rate = state.error_count as f64 / total as f64;

            if error_rate > 0.1 {
                state.current_rate = (state.current_rate * 0.9).max(self.min_rate);
            } else if error_rate < 0.01 {
                state.current_rate = (state.current_rate * 1.1).min(self.max_rate);
            }

            let new_limiter = TokenBucket::new(state.current_rate as usize, state.current_rate);
            state.inner = AnyRateLimiter::TokenBucket(Arc::new(new_limiter));
        }

        state.success_count = 0;
        state.error_count = 0;
        state.last_stats_reset = Instant::now();
    }

    /// Record success
    pub async fn record_success(&self) {
        let mut state = self.state.lock().await;
        state.success_count += 1;
        self.adjust_rate_locked(&mut state);
    }

    /// Record error
    pub async fn record_error(&self) {
        let mut state = self.state.lock().await;
        state.error_count += 1;
        self.adjust_rate_locked(&mut state);
    }
}

#[async_trait]
impl RateLimiter for AdaptiveRateLimiter {
    async fn acquire(&self) -> ResilienceResult<()> {
        self.state.lock().await.inner.acquire().await
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
        self.state.lock().await.current_rate
    }

    async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.inner.reset().await;
        state.success_count = 0;
        state.error_count = 0;
    }
}
