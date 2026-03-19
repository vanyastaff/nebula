//! Sliding window rate limiter implementation

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::RateLimiter;
use crate::{ResilienceError, ResilienceResult};

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

    fn clean_old_requests_locked(
        requests: &mut VecDeque<Instant>,
        now: Instant,
        window_duration: Duration,
    ) {
        let cutoff = now.checked_sub(window_duration).unwrap_or(now);

        while let Some(&front) = requests.front() {
            if front < cutoff {
                requests.pop_front();
            } else {
                break;
            }
        }
    }
}

impl RateLimiter for SlidingWindow {
    async fn acquire(&self) -> ResilienceResult<()> {
        let now = Instant::now();
        let mut requests = self.requests.lock();
        Self::clean_old_requests_locked(&mut requests, now, self.window_duration);

        if requests.len() < self.max_requests {
            requests.push_back(now);
            drop(requests);
            Ok(())
        } else {
            // Calculate retry after based on oldest request
            let retry_after = requests.front().map_or(Duration::from_secs(1), |&oldest| {
                self.window_duration
                    .checked_sub(now.duration_since(oldest))
                    .unwrap_or(Duration::from_millis(1))
            });
            drop(requests);

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
        let now = Instant::now();
        let mut requests = self.requests.lock();
        Self::clean_old_requests_locked(&mut requests, now, self.window_duration);
        let len = requests.len() as f64;
        drop(requests);
        len
    }

    async fn reset(&self) {
        let mut requests = self.requests.lock();
        requests.clear();
    }
}
