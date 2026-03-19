//! Leaky bucket rate limiter implementation

use parking_lot::Mutex;
use std::future::Future;
use std::time::Instant;

use super::RateLimiter;
use crate::CallError;

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

impl RateLimiter for LeakyBucket {
    async fn acquire(&self) -> Result<(), CallError<()>> {
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
            Err(CallError::RateLimited)
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
        let state = self.state.lock();
        (self.capacity - state.level) as f64
    }

    async fn reset(&self) {
        let mut state = self.state.lock();
        state.level = 0;
        state.last_leak = Instant::now();
    }
}
