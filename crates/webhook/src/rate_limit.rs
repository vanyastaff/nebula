//! Per-path webhook rate limiting.
//!
//! Provides a simple fixed-window rate limiter that tracks request
//! counts per webhook path. Designed for protecting webhook endpoints
//! from excessive traffic.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::error::Error;

/// Per-path rate limiter using a fixed-window counter.
///
/// Each unique webhook path gets an independent counter that resets
/// after the configured window duration (default: 60 seconds).
///
/// # Examples
///
/// ```
/// use nebula_webhook::WebhookRateLimiter;
///
/// let limiter = WebhookRateLimiter::new(100); // 100 RPM per path
/// assert!(limiter.check("/hooks/abc123").is_ok());
/// ```
#[derive(Debug)]
pub struct WebhookRateLimiter {
    /// Per-path counters: path -> (count, window_start).
    counters: DashMap<String, RateCounter>,
    /// Maximum requests per window.
    max_requests: u64,
    /// Window duration.
    window: Duration,
}

#[derive(Debug)]
struct RateCounter {
    count: AtomicU64,
    window_start: Instant,
}

impl WebhookRateLimiter {
    /// Create a rate limiter with the given requests-per-minute limit.
    ///
    /// Each webhook path is tracked independently.
    #[must_use]
    pub fn new(requests_per_minute: u64) -> Self {
        Self {
            counters: DashMap::new(),
            max_requests: requests_per_minute,
            window: Duration::from_secs(60),
        }
    }

    /// Check if a request to the given path is allowed.
    ///
    /// Increments the counter for the path and returns `Ok(())` if
    /// the request is within the rate limit.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RateLimited`] if the path has exceeded its
    /// per-minute request quota.
    pub fn check(&self, path: &str) -> Result<(), Error> {
        let now = Instant::now();

        let mut entry = self
            .counters
            .entry(path.to_string())
            .or_insert_with(|| RateCounter {
                count: AtomicU64::new(0),
                window_start: now,
            });

        // Reset window if expired
        if now.duration_since(entry.window_start) >= self.window {
            entry.count.store(0, Ordering::Relaxed);
            entry.window_start = now;
        }

        let current = entry.count.fetch_add(1, Ordering::Relaxed);
        if current >= self.max_requests {
            // Rollback the increment
            entry.count.fetch_sub(1, Ordering::Relaxed);
            let retry_after = self
                .window
                .checked_sub(now.duration_since(entry.window_start))
                .unwrap_or(self.window)
                .as_secs();
            return Err(Error::rate_limited(path, retry_after));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_within_limit() {
        let limiter = WebhookRateLimiter::new(10);
        for _ in 0..10 {
            assert!(limiter.check("/test").is_ok());
        }
    }

    #[test]
    fn rejects_over_limit() {
        let limiter = WebhookRateLimiter::new(5);
        for _ in 0..5 {
            assert!(limiter.check("/test").is_ok());
        }
        let result = limiter.check("/test");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), Error::RateLimited { .. }));
    }

    #[test]
    fn tracks_paths_independently() {
        let limiter = WebhookRateLimiter::new(2);
        assert!(limiter.check("/a").is_ok());
        assert!(limiter.check("/a").is_ok());
        assert!(limiter.check("/a").is_err());
        // Different path has a fresh counter
        assert!(limiter.check("/b").is_ok());
    }

    #[test]
    fn rate_limited_error_includes_path() {
        let limiter = WebhookRateLimiter::new(1);
        assert!(limiter.check("/my-hook").is_ok());
        let err = limiter.check("/my-hook").unwrap_err();
        match err {
            Error::RateLimited { path, .. } => {
                assert_eq!(path, "/my-hook");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn single_request_allowed_with_limit_one() {
        let limiter = WebhookRateLimiter::new(1);
        assert!(limiter.check("/x").is_ok());
        assert!(limiter.check("/x").is_err());
    }
}
