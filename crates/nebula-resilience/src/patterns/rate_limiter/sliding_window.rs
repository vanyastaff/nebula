//! Sliding window rate limiter implementation

use async_trait::async_trait;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::{ResilienceError, ResilienceResult};
use super::RateLimiter;

/// Sliding window rate limiter
///
/// Tracks requests in a sliding time window and limits based on count.
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
        let cutoff = Instant::now()
            .checked_sub(self.window_duration)
            .expect("Window duration exceeds Instant range");

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
            let retry_after = requests.front().map_or(Duration::from_secs(1), |&oldest| {
                self.window_duration
                    .checked_sub(oldest.elapsed())
                    .unwrap_or(Duration::from_millis(1))
            });

            Err(ResilienceError::RateLimitExceeded {
                retry_after: Some(retry_after),
                limit: self.max_requests as f64,
                current: (self.max_requests + 1) as f64,
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
