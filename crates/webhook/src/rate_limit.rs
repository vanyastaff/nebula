//! Per-path webhook rate limiting.
//!
//! Wraps [`nebula_resilience::SlidingWindow`] with per-path tracking.
//! Each unique webhook path gets an independent sliding window limiter.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use nebula_resilience::SlidingWindow;

use crate::error::Error;

/// Per-path rate limiter backed by [`SlidingWindow`] from nebula-resilience.
///
/// Each unique webhook path gets an independent sliding window that
/// tracks request timestamps and enforces requests-per-minute limits.
///
/// # Examples
///
/// ```rust,ignore
/// let limiter = WebhookRateLimiter::new(100); // 100 RPM per path
/// assert!(limiter.check("/hooks/abc123").await.is_ok());
/// ```
#[derive(Debug)]
pub struct WebhookRateLimiter {
    /// Per-path sliding windows.
    windows: DashMap<String, Arc<SlidingWindow>>,
    /// Maximum requests per window.
    max_requests: usize,
    /// Window duration.
    window: Duration,
}

impl WebhookRateLimiter {
    /// Create a rate limiter with the given requests-per-minute limit.
    ///
    /// Each webhook path is tracked independently using a sliding window.
    #[must_use]
    pub fn new(requests_per_minute: u64) -> Self {
        Self {
            windows: DashMap::new(),
            max_requests: requests_per_minute.max(1) as usize,
            window: Duration::from_secs(60),
        }
    }

    /// Check if a request to the given path is allowed.
    ///
    /// Uses a sliding window per path — more accurate than fixed-window
    /// counters at window boundaries.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RateLimited`] if the path has exceeded its
    /// per-minute request quota.
    pub async fn check(&self, path: &str) -> Result<(), Error> {
        let window = self
            .windows
            .entry(path.to_string())
            .or_insert_with(|| {
                // SlidingWindow::new validates inputs — our constructor
                // already ensures max_requests >= 1 and window > 0.
                Arc::new(
                    SlidingWindow::new(self.window, self.max_requests)
                        .expect("valid config: max_requests >= 1, window > 0"),
                )
            })
            .clone();

        use nebula_resilience::RateLimiter;
        match window.acquire().await {
            Ok(()) => Ok(()),
            Err(_) => {
                // SlidingWindow doesn't provide retry_after, estimate it
                let retry_after = self.window.as_secs();
                Err(Error::rate_limited(path, retry_after))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allows_within_limit() {
        let limiter = WebhookRateLimiter::new(10);
        for _ in 0..10 {
            assert!(limiter.check("/test").await.is_ok());
        }
    }

    #[tokio::test]
    async fn rejects_over_limit() {
        let limiter = WebhookRateLimiter::new(5);
        for _ in 0..5 {
            assert!(limiter.check("/test").await.is_ok());
        }
        let result = limiter.check("/test").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RateLimited { .. }));
    }

    #[tokio::test]
    async fn tracks_paths_independently() {
        let limiter = WebhookRateLimiter::new(2);
        assert!(limiter.check("/a").await.is_ok());
        assert!(limiter.check("/a").await.is_ok());
        assert!(limiter.check("/a").await.is_err());
        // Different path has a fresh counter
        assert!(limiter.check("/b").await.is_ok());
    }

    #[tokio::test]
    async fn rate_limited_error_includes_path() {
        let limiter = WebhookRateLimiter::new(1);
        assert!(limiter.check("/my-hook").await.is_ok());
        let err = limiter.check("/my-hook").await.unwrap_err();
        match err {
            Error::RateLimited { path, .. } => {
                assert_eq!(path, "/my-hook");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn single_request_allowed_with_limit_one() {
        let limiter = WebhookRateLimiter::new(1);
        assert!(limiter.check("/x").await.is_ok());
        assert!(limiter.check("/x").await.is_err());
    }
}
