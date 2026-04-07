//! Per-path webhook rate limiting.
//!
//! Wraps [`nebula_resilience::SlidingWindow`] with per-path tracking.
//! Each unique webhook path gets an independent sliding window limiter.
//!
//! To prevent memory exhaustion from attacker-controlled paths, the limiter
//! enforces a maximum number of tracked paths (`max_paths`). Requests for
//! paths beyond this limit pass through without per-path rate limiting.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use nebula_resilience::SlidingWindow;

use crate::error::Error;

/// Default maximum number of distinct paths to track.
///
/// Paths beyond this limit are allowed without per-path rate limiting
/// to prevent unbounded `DashMap` growth.
const DEFAULT_MAX_PATHS: usize = 10_000;

/// Per-path rate limiter backed by [`SlidingWindow`] from nebula-resilience.
///
/// Each unique webhook path gets an independent sliding window that
/// tracks request timestamps and enforces requests-per-minute limits.
///
/// A `max_paths` cap (default: 10 000) prevents the internal map from
/// growing without bound when callers supply attacker-controlled paths.
/// Requests arriving on previously-unseen paths above the cap are allowed
/// through without per-path rate limiting.
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
    /// Maximum number of distinct paths to track.
    max_paths: usize,
}

impl WebhookRateLimiter {
    /// Create a rate limiter with the given requests-per-minute limit.
    ///
    /// Each webhook path is tracked independently using a sliding window.
    /// At most [`DEFAULT_MAX_PATHS`] distinct paths are tracked; paths
    /// beyond that cap are passed through without rate limiting.
    #[must_use]
    pub fn new(requests_per_minute: u64) -> Self {
        Self {
            windows: DashMap::new(),
            max_requests: requests_per_minute.max(1) as usize,
            window: Duration::from_secs(60),
            max_paths: DEFAULT_MAX_PATHS,
        }
    }

    /// Override the maximum number of distinct paths to track.
    ///
    /// Paths beyond this limit are passed through without rate limiting
    /// rather than causing unbounded allocations.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_paths(mut self, max_paths: usize) -> Self {
        self.max_paths = max_paths.max(1);
        self
    }

    /// Check if a request to the given path is allowed.
    ///
    /// Uses a sliding window per path — more accurate than fixed-window
    /// counters at window boundaries.
    ///
    /// If the path has not been seen before and the number of tracked paths
    /// has already reached `max_paths`, the request is allowed through
    /// without per-path rate limiting to prevent memory exhaustion.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RateLimited`] if the path has exceeded its
    /// per-minute request quota.
    pub async fn check(&self, path: &str) -> Result<(), Error> {
        // Avoid inserting new entries once the cap is reached.
        if !self.windows.contains_key(path) && self.windows.len() >= self.max_paths {
            return Ok(());
        }

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

    #[tokio::test]
    async fn paths_beyond_capacity_are_passed_through() {
        // Cap at 2 paths; the 3rd unique path must be allowed (not tracked).
        let limiter = WebhookRateLimiter::new(1).with_max_paths(2);
        // Exhaust the two tracked slots
        assert!(limiter.check("/a").await.is_ok());
        assert!(limiter.check("/b").await.is_ok());
        // /a and /b are now rate-limited
        assert!(limiter.check("/a").await.is_err());
        assert!(limiter.check("/b").await.is_err());
        // /c is a new path but capacity is reached — passes through
        assert!(limiter.check("/c").await.is_ok());
        // Repeated calls on /c also pass through (not tracked)
        assert!(limiter.check("/c").await.is_ok());
    }
}
