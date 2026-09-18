//! Sliding-window rate limiter.

use std::{
    collections::VecDeque,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::CallError;

use super::{RateLimiter, RateLimiterStatus, rate_limited_with_retry_after};
/// Rate limiter based on a **sliding time window** counter.
///
/// Maintains a timestamped log of recent requests. On each `acquire()` call
/// stale entries (older than `window_duration`) are evicted; the call succeeds
/// only when the number of remaining entries is below `max_requests`.
///
/// # Configuration
///
/// - `window_duration` — rolling window length (must be `> 0`).
/// - `max_requests` — maximum allowed requests within any window (must be `≥ 1`).
///
/// # When to choose this
///
/// Use [`SlidingWindow`] when you need a strict per-window request cap that
/// avoids the boundary burst problem of fixed windows — for example,
/// enforcing "at most 100 calls per minute" with no double-counting at the
/// minute boundary. The trade-off is O(N) memory proportional to
/// `max_requests`.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::{RateLimiter, rate_limiter::SlidingWindow};
///
/// # #[tokio::main]
/// # async fn main() {
/// // At most 100 acquisitions in any rolling 1-minute window.
/// let limiter = SlidingWindow::new(Duration::from_secs(60), 100).expect("valid config");
///
/// limiter.acquire().await.expect("under cap");
/// # }
/// ```
pub struct SlidingWindow {
    /// Window duration
    window_duration: Duration,
    /// Maximum requests per window
    max_requests: usize,
    /// Request timestamps
    requests: Arc<Mutex<VecDeque<Instant>>>,
}

impl fmt::Debug for SlidingWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlidingWindow")
            .field("window_duration", &self.window_duration)
            .field("max_requests", &self.max_requests)
            .finish_non_exhaustive()
    }
}

impl SlidingWindow {
    /// Creates a new sliding window rate limiter.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `max_requests` is 0
    /// or `window_duration` is zero.
    pub fn new(window_duration: Duration, max_requests: usize) -> Result<Self, crate::ConfigError> {
        if max_requests == 0 {
            return Err(crate::ConfigError::new("max_requests", "must be >= 1"));
        }
        if window_duration.is_zero() {
            return Err(crate::ConfigError::new("window_duration", "must be > 0"));
        }
        Ok(Self {
            window_duration,
            max_requests,
            requests: Arc::new(Mutex::new(VecDeque::with_capacity(max_requests))),
        })
    }

    fn clean_old_requests_locked(requests: &mut VecDeque<Instant>, cutoff: Instant) {
        while let Some(&front) = requests.front() {
            if front <= cutoff {
                requests.pop_front();
            } else {
                break;
            }
        }
    }

    fn retry_after_locked(
        requests: &VecDeque<Instant>,
        window_duration: Duration,
        now: Instant,
    ) -> Option<Duration> {
        let oldest = *requests.front()?;
        let expires_at = oldest.checked_add(window_duration)?;
        Some(
            expires_at
                .checked_duration_since(now)
                .unwrap_or(Duration::ZERO),
        )
    }
}

impl RateLimiter for SlidingWindow {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let now = Instant::now();
        let cutoff = now.checked_sub(self.window_duration).unwrap_or(now);
        let mut requests = self.requests.lock();

        // Always evict expired entries before checking capacity.
        // The deque is sorted by insertion time, so we only scan from the
        // front until we hit a non-expired entry — O(k) where k is the
        // number of expired entries (typically 0–1 at steady-state).
        Self::clean_old_requests_locked(&mut requests, cutoff);

        if requests.len() < self.max_requests {
            requests.push_back(now);
            drop(requests);
            Ok(())
        } else {
            let retry_after = Self::retry_after_locked(&requests, self.window_duration, now);
            drop(requests);
            Err(rate_limited_with_retry_after(retry_after))
        }
    }

    // Reason: usize request count cast to f64 — acceptable for rate reporting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize request count cast to f64 — acceptable for rate reporting"
    )]
    async fn status(&self) -> RateLimiterStatus {
        let now = Instant::now();
        let mut requests = self.requests.lock();
        // Always do a full cleanup here so the reported count is accurate.
        let cutoff = now.checked_sub(self.window_duration).unwrap_or(now);
        Self::clean_old_requests_locked(&mut requests, cutoff);
        let used = requests.len();
        drop(requests);
        // Remaining window quota: a window counter enforces a count per
        // window, so it reports quota and no rate.
        RateLimiterStatus::new(self.max_requests.saturating_sub(used) as f64, None)
    }

    async fn reset(&self) {
        let mut requests = self.requests.lock();
        requests.clear();
    }
}
