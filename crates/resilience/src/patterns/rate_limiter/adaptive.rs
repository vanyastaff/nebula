//! Adaptive rate limiter that adjusts based on error rates

use parking_lot::RwLock;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::{RateLimiter, TokenBucket};
use crate::CallError;

/// Mutable state behind a single lock.
struct AdaptiveState {
    inner: Arc<TokenBucket>,
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
    state: Arc<RwLock<AdaptiveState>>,
    /// Lock-free copy of `current_rate` for cheap reads without taking the lock.
    /// Stored as `f64::to_bits()` / read via `f64::from_bits()`.
    atomic_rate: AtomicU64,
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
            state: Arc::new(RwLock::new(AdaptiveState {
                inner: Arc::new(token_bucket),
                success_count: 0,
                error_count: 0,
                last_stats_reset: Instant::now(),
                current_rate: initial_rate,
            })),
            atomic_rate: AtomicU64::new(initial_rate.to_bits()),
            stats_window: Duration::from_mins(1),
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
            state.inner = Arc::new(new_limiter);
            self.atomic_rate
                .store(state.current_rate.to_bits(), Ordering::Release);
        }

        state.success_count = 0;
        state.error_count = 0;
        state.last_stats_reset = Instant::now();
    }

    /// Record success
    pub fn record_success(&self) {
        let mut state = self.state.write();
        state.success_count += 1;
        self.adjust_rate_locked(&mut state);
        drop(state);
    }

    /// Record error
    pub fn record_error(&self) {
        let mut state = self.state.write();
        state.error_count += 1;
        self.adjust_rate_locked(&mut state);
        drop(state);
    }
}

impl RateLimiter for AdaptiveRateLimiter {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let limiter = {
            let state = self.state.read();
            state.inner.clone()
        };

        limiter.acquire().await
    }

    async fn execute<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send,
    {
        self.acquire().await.map_err(|_| CallError::RateLimited)?;
        let result = operation().await;

        match &result {
            Ok(_) => self.record_success(),
            Err(_) => self.record_error(),
        }

        result.map_err(CallError::Operation)
    }

    async fn current_rate(&self) -> f64 {
        f64::from_bits(self.atomic_rate.load(Ordering::Acquire))
    }

    async fn reset(&self) {
        // Reset stats and snapshot the current inner limiter atomically under a single write lock.
        // Splitting into read-then-write would be a TOCTOU: a concurrent record_error() could
        // install a new inner bucket between the two lock acquisitions.
        let limiter = {
            let mut state = self.state.write();
            state.success_count = 0;
            state.error_count = 0;
            state.last_stats_reset = Instant::now();
            state.inner.clone()
        };

        limiter.reset().await;
    }
}
