//! Per-path webhook rate limiting.
//!
//! Salvaged from the deleted `crates/webhook/` orphan (verbatim port
//! of `rate_limit.rs`) and adapted to use a local error type instead
//! of the old crate's `Error` enum. Wraps
//! [`nebula_resilience::SlidingWindow`] with per-path tracking:
//! each unique webhook path gets an independent sliding window
//! limiter.
//!
//! To prevent memory exhaustion from attacker-controlled paths, the
//! limiter enforces a maximum number of tracked paths (`max_paths`).
//! Requests for paths beyond this limit pass through without
//! per-path rate limiting — the soft cap keeps `DashMap` bounded.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use dashmap::DashMap;
use nebula_resilience::SlidingWindow;

/// Default maximum number of distinct paths to track.
///
/// Paths beyond this limit are allowed without per-path rate limiting
/// to prevent unbounded `DashMap` growth.
const DEFAULT_MAX_PATHS: usize = 10_000;

/// Result of a `path_count` slot reservation attempt.
enum SlotReservation {
    Reserved,
    Saturated,
}

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
/// Each unique webhook path gets an independent sliding window that
/// tracks request timestamps and enforces requests-per-minute limits.
///
/// A `max_paths` cap (default: 10 000) prevents the internal map from
/// growing without bound when callers supply attacker-controlled paths.
/// Requests arriving on previously-unseen paths above the cap are allowed
/// through without per-path rate limiting.
///
/// The cap is a **soft limit**: in high concurrency scenarios, the map
/// may briefly exceed `max_paths` by a small margin before the atomic
/// counter catches up. This is acceptable for DoS-prevention purposes.
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
    /// Atomic count of distinct paths currently tracked.
    path_count: AtomicUsize,
}

impl WebhookRateLimiter {
    /// Create a rate limiter with the given requests-per-minute limit.
    #[must_use]
    pub fn new(requests_per_minute: u64) -> Self {
        Self {
            windows: DashMap::new(),
            max_requests: requests_per_minute.max(1) as usize,
            window: Duration::from_secs(60),
            max_paths: DEFAULT_MAX_PATHS,
            path_count: AtomicUsize::new(0),
        }
    }

    /// Override the maximum number of distinct paths to track.
    #[must_use]
    pub fn with_max_paths(mut self, max_paths: usize) -> Self {
        self.max_paths = max_paths.max(1);
        self
    }

    /// Check if a request to the given path is allowed.
    ///
    /// Uses a sliding window per path — more accurate than fixed-window
    /// counters at window boundaries.
    ///
    /// If the path has not been seen before and the number of tracked
    /// paths has already reached `max_paths`, the request is allowed
    /// through without per-path rate limiting to prevent memory
    /// exhaustion.
    ///
    /// # Errors
    ///
    /// Returns [`RateLimitExceeded`] if the path has exceeded its
    /// per-minute request quota.
    pub async fn check(&self, path: &str) -> Result<(), RateLimitExceeded> {
        // Fast path: path is already tracked — no allocation.
        if let Some(window) = self.windows.get(path) {
            return Self::acquire(window.clone(), path, self.window.as_secs()).await;
        }

        // Slow path: get or insert the window atomically via DashMap's
        // `entry` API. `path_count` is bumped only inside the `Vacant`
        // arm (via `try_reserve_slot`) so two concurrent first-time
        // requests for the same path insert exactly one entry and
        // increment exactly once — the previous pre-CAS-then-entry
        // ordering could overcount `path_count` and eventually cross
        // `max_paths` silently, disabling per-path limiting wholesale.
        use dashmap::mapref::entry::Entry;
        let window = match self.windows.entry(path.to_string()) {
            Entry::Occupied(e) => e.get().clone(),
            Entry::Vacant(v) => match self.try_reserve_slot() {
                SlotReservation::Reserved => v
                    .insert(Arc::new(
                        SlidingWindow::new(self.window, self.max_requests)
                            .expect("valid config: max_requests >= 1, window > 0"),
                    ))
                    .clone(),
                SlotReservation::Saturated => return Ok(()),
            },
        };

        Self::acquire(window, path, self.window.as_secs()).await
    }

    /// Attempts to reserve a slot in the `path_count` soft cap.
    ///
    /// Returns `Reserved` if `path_count` was bumped under the cap;
    /// `Saturated` if the map has already reached `max_paths` and the
    /// caller must pass the request through without tracking.
    fn try_reserve_slot(&self) -> SlotReservation {
        let mut current = self.path_count.load(Ordering::Relaxed);
        loop {
            if current >= self.max_paths {
                return SlotReservation::Saturated;
            }
            match self.path_count.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return SlotReservation::Reserved,
                Err(actual) => current = actual,
            }
        }
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

    #[tokio::test]
    async fn paths_beyond_capacity_are_passed_through() {
        let limiter = WebhookRateLimiter::new(1).with_max_paths(2);
        assert!(limiter.check("/a").await.is_ok());
        assert!(limiter.check("/b").await.is_ok());
        assert!(limiter.check("/a").await.is_err());
        assert!(limiter.check("/b").await.is_err());
        // /c is a new path but capacity is reached — passes through.
        assert!(limiter.check("/c").await.is_ok());
        assert!(limiter.check("/c").await.is_ok());
    }
}
