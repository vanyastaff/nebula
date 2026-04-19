//! Per-path webhook rate limiting.
//!
//! Wraps [`nebula_resilience::SlidingWindow`] with per-path tracking: each
//! unique webhook path gets an independent sliding-window limiter.
//!
//! To prevent memory exhaustion from attacker-controlled paths, the limiter
//! caps the number of tracked paths (`max_paths`) using an LRU-style bound
//! via [`moka::future::Cache`]. When the cap is reached, the **least-recently
//! used** window is evicted to make room for the new path — so rate-limiting
//! stays enforced for every path (#271). The previous implementation let
//! requests for untracked paths fall through without any limit once the cap
//! was hit, which allowed an attacker who probed enough unique paths to
//! permanently disable per-path limiting for newly-deployed routes.

use std::{sync::Arc, time::Duration};

use moka::future::Cache;
use nebula_resilience::SlidingWindow;

/// Default maximum number of distinct paths to track.
///
/// Paths beyond this limit are retained via LRU eviction — the oldest path
/// is dropped so new arrivals always receive a tracked window.
const DEFAULT_MAX_PATHS: u64 = 10_000;

/// Error returned when a request exceeds the per-path quota.
#[derive(Debug, Clone, thiserror::Error)]
#[error("webhook rate limit exceeded for path {path:?} (retry after {retry_after_secs}s)")]
pub struct RateLimitExceeded {
    /// Rate-limited path.
    pub path: String,
    /// Suggested `Retry-After` value in seconds.
    pub retry_after_secs: u64,
}

/// Per-path rate limiter backed by [`SlidingWindow`] from nebula-resilience.
///
/// Each unique webhook path gets an independent sliding window that tracks
/// request timestamps and enforces requests-per-minute limits.
///
/// The `max_paths` cap (default: 10 000) bounds memory via **LRU eviction**:
/// when a new path arrives after the cap is reached, the least-recently used
/// window is dropped. Every path — attacker-controlled or legitimate — is
/// rate-limited the same way; the cap does not create a bypass. A window that
/// has been idle for longer than twice the request window is also eligible
/// for eviction so paths decommissioned by operators do not linger.
#[derive(Debug, Clone)]
pub struct WebhookRateLimiter {
    windows: Cache<String, Arc<SlidingWindow>>,
    max_requests: usize,
    window: Duration,
}

impl WebhookRateLimiter {
    /// Create a rate limiter with the given requests-per-minute limit.
    #[must_use]
    pub fn new(requests_per_minute: u64) -> Self {
        Self::with_config(requests_per_minute, DEFAULT_MAX_PATHS)
    }

    /// Override the maximum number of distinct paths to track.
    #[must_use]
    pub fn with_max_paths(self, max_paths: usize) -> Self {
        let rpm = self.max_requests as u64;
        Self::with_config(rpm, (max_paths.max(1)) as u64)
    }

    fn with_config(requests_per_minute: u64, max_paths: u64) -> Self {
        let window = Duration::from_secs(60);
        let max_requests = requests_per_minute.max(1) as usize;
        let windows = Cache::builder()
            .max_capacity(max_paths.max(1))
            .time_to_idle(window.saturating_mul(2))
            .build();
        Self {
            windows,
            max_requests,
            window,
        }
    }

    /// Check if a request to the given path is allowed.
    ///
    /// Uses a sliding window per path — more accurate than fixed-window
    /// counters at window boundaries. If the path is not tracked yet a fresh
    /// window is inserted, evicting the least-recently used entry when the
    /// cap is reached.
    ///
    /// # Errors
    ///
    /// Returns [`RateLimitExceeded`] if the path has exceeded its
    /// per-minute request quota.
    pub async fn check(&self, path: &str) -> Result<(), RateLimitExceeded> {
        let window_dur = self.window;
        let max_requests = self.max_requests;

        let window = self
            .windows
            .get_with(path.to_string(), async move {
                Arc::new(
                    SlidingWindow::new(window_dur, max_requests)
                        .expect("valid config: max_requests >= 1, window > 0"),
                )
            })
            .await;

        Self::acquire(window, path, window_dur.as_secs()).await
    }

    async fn acquire(
        window: Arc<SlidingWindow>,
        path: &str,
        retry_after_secs: u64,
    ) -> Result<(), RateLimitExceeded> {
        use nebula_resilience::RateLimiter;
        match window.acquire().await {
            Ok(()) => Ok(()),
            Err(_) => Err(RateLimitExceeded {
                path: path.to_string(),
                retry_after_secs,
            }),
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
        assert!(limiter.check("/test").await.is_err());
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
    async fn rate_limited_error_includes_path_and_retry_after() {
        let limiter = WebhookRateLimiter::new(1);
        assert!(limiter.check("/my-hook").await.is_ok());
        let err = limiter.check("/my-hook").await.unwrap_err();
        assert_eq!(err.path, "/my-hook");
        assert_eq!(err.retry_after_secs, 60);
    }

    /// Regression for #271: a new path that arrives after the capacity cap
    /// has been reached must still be rate-limited. The previous
    /// implementation passed every over-cap path through with `Ok(())`,
    /// giving an attacker who probed `max_paths` unique paths a permanent
    /// bypass for every subsequent path.
    #[tokio::test]
    async fn new_path_beyond_capacity_is_still_rate_limited() {
        let limiter = WebhookRateLimiter::new(1).with_max_paths(2);

        // Saturate the existing paths within their own quotas.
        assert!(limiter.check("/a").await.is_ok());
        assert!(limiter.check("/a").await.is_err());
        assert!(limiter.check("/b").await.is_ok());
        assert!(limiter.check("/b").await.is_err());

        // A brand-new path pushes capacity over the cap. It must get its
        // own tracked window (one allowed, then blocked), not a free pass.
        assert!(limiter.check("/c").await.is_ok());
        assert!(
            limiter.check("/c").await.is_err(),
            "new over-cap path must be rate-limited, not bypassed",
        );
    }

    /// Regression for #271: attacker churn through many unique paths must
    /// not permanently fill slots. LRU eviction drops the oldest entries so
    /// legitimate new paths keep getting tracked windows.
    #[tokio::test]
    async fn attacker_churn_does_not_exhaust_limiter() {
        let limiter = WebhookRateLimiter::new(1).with_max_paths(4);

        // Attacker probes far more unique paths than the cap.
        for i in 0..100 {
            let path = format!("/attacker-{i}");
            // First hit for each path is allowed; second would be blocked.
            assert!(limiter.check(&path).await.is_ok());
        }

        // A legitimate new path must still receive a tracked window.
        assert!(limiter.check("/legit").await.is_ok());
        assert!(
            limiter.check("/legit").await.is_err(),
            "legitimate path must be rate-limited after the attacker flood",
        );
    }
}
