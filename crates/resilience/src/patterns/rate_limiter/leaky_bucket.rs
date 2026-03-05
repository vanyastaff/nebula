//! Leaky bucket rate limiter implementation

use async_trait::async_trait;
use parking_lot::Mutex;
use std::future::Future;
use std::time::{Duration, Instant};

use super::RateLimiter;
use crate::{ResilienceError, ResilienceResult};

#[derive(Debug)]
struct LeakyBucketState {
    level: usize,
    last_leak: Instant,
}

/// Leaky bucket rate limiter
///
/// Implements the leaky bucket algorithm where requests fill a bucket
/// that leaks at a constant rate.
pub struct LeakyBucket {
    /// Bucket capacity
    capacity: usize,
    /// Mutable runtime state
    state: Mutex<LeakyBucketState>,
    /// Leak rate per second
    leak_rate: f64,
}

impl LeakyBucket {
    /// Create new leaky bucket
    #[must_use]
    pub fn new(capacity: usize, leak_rate: f64) -> Self {
        Self {
            capacity,
            state: Mutex::new(LeakyBucketState {
                level: 0,
                last_leak: Instant::now(),
            }),
            leak_rate,
        }
    }
}

#[async_trait]
impl RateLimiter for LeakyBucket {
    async fn acquire(&self) -> ResilienceResult<()> {
        let mut state = self.state.lock();

        let now = Instant::now();
        let elapsed = now.duration_since(state.last_leak).as_secs_f64();
        let leaked = (elapsed * self.leak_rate) as usize;
        state.level = state.level.saturating_sub(leaked);
        state.last_leak = now;

        if state.level < self.capacity {
            state.level += 1;
            drop(state);
            Ok(())
        } else {
            drop(state);
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
        let state = self.state.lock();
        (self.capacity - state.level) as f64
    }

    async fn reset(&self) {
        let mut state = self.state.lock();
        state.level = 0;
        state.last_leak = Instant::now();
    }
}
