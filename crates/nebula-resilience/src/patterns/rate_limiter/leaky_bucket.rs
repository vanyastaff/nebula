//! Leaky bucket rate limiter implementation

use async_trait::async_trait;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::RateLimiter;
use crate::{ResilienceError, ResilienceResult};

/// Leaky bucket rate limiter
///
/// Implements the leaky bucket algorithm where requests fill a bucket
/// that leaks at a constant rate.
pub struct LeakyBucket {
    /// Bucket capacity
    capacity: usize,
    /// Current water level
    level: Arc<Mutex<usize>>,
    /// Leak rate per second
    leak_rate: f64,
    /// Last leak timestamp
    last_leak: Arc<Mutex<Instant>>,
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
        drop(level);
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
            drop(level);
            Ok(())
        } else {
            drop(level);
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
