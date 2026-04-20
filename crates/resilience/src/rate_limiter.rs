//! Rate limiting implementations.
//!
//! This module provides multiple rate limiting algorithms:
//!
//! - **`TokenBucket`**: Classic token bucket with refill rate
//! - **`LeakyBucket`**: Leaky bucket with constant leak rate
//! - **`SlidingWindow`**: Sliding time window counter
//! - **`AdaptiveRateLimiter`**: Self-adjusting based on error rates
//! - **`GovernorRateLimiter`**: Production-grade GCRA algorithm
//!
//! # Examples
//!
//! ```
//! use nebula_resilience::rate_limiter::TokenBucket;
//!
//! let limiter = TokenBucket::new(100, 10.0).unwrap();
//! ```

use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::{Mutex, RwLock};

use crate::CallError;

// ═══════════════════════════════════════════════════════════════════════════════
// TRAIT
// ═══════════════════════════════════════════════════════════════════════════════

/// Rate limiter trait.
///
/// Returns `Err(CallError::RateLimited)` when the rate limit is exceeded.
///
/// All async methods return `Send` futures, matching the `Send + Sync` bound on the trait.
///
/// This trait is designed to be implemented by downstream crates.
/// New methods will always have default implementations to avoid breaking changes.
pub trait RateLimiter: Send + Sync {
    /// Try to acquire permission. Returns `Err(CallError::RateLimited)` if limit hit.
    fn acquire(&self) -> impl Future<Output = Result<(), CallError<()>>> + Send;

    /// Acquire permission then call `operation`. Returns `Err(CallError::RateLimited)` or
    /// the operation's own error wrapped in `CallError::Operation`.
    ///
    /// Default implementation calls [`acquire`](Self::acquire) then the operation.
    /// Override only if you need custom behavior (e.g., recording success/error).
    fn call<T, E, F, Fut>(
        &self,
        operation: F,
    ) -> impl Future<Output = Result<T, CallError<E>>> + Send
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send,
    {
        async {
            self.acquire()
                .await
                .map_err(|_| CallError::rate_limited())?;
            operation().await.map_err(CallError::Operation)
        }
    }

    /// Returns the current rate or available capacity (implementation-dependent).
    fn current_rate(&self) -> impl Future<Output = f64> + Send;

    /// Clears all state and resets to initial conditions.
    fn reset(&self) -> impl Future<Output = ()> + Send;
}

// ═══════════════════════════════════════════════════════════════════════════════
// TOKEN BUCKET
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug)]
struct TokenBucketState {
    tokens: f64,
    last_refill: Instant,
}

/// Token bucket rate limiter.
///
/// Classic token bucket algorithm with configurable capacity and refill rate.
/// Tokens are added at a constant rate and consumed by operations.
///
/// # Examples
///
/// ```
/// use nebula_resilience::rate_limiter::TokenBucket;
///
/// let limiter = TokenBucket::new(100, 10.0).unwrap();
/// ```
pub struct TokenBucket {
    /// Maximum tokens in bucket (initial value, used by `reset`).
    capacity: usize,
    /// Mutable runtime state
    state: Mutex<TokenBucketState>,
    /// Token refill rate per second — stored atomically for lock-free reads
    /// and updated in-place by `update_rate` to avoid re-allocation.
    refill_rate: AtomicU64,
    /// Burst size — the live cap on accumulated tokens.
    /// Stored atomically so it can be updated alongside `refill_rate` by
    /// the adaptive rate limiter without rebuilding the `TokenBucket`.
    burst_size: AtomicUsize,
}

impl fmt::Debug for TokenBucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenBucket")
            .field("capacity", &self.capacity)
            .field("refill_rate", &self.refill_rate)
            .field("burst_size", &self.burst_size)
            .finish_non_exhaustive()
    }
}

impl TokenBucket {
    /// Create new token bucket.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `capacity` is 0 or > 100,000,
    /// or `refill_rate` is outside 0.001..=10,000.0.
    // Reason: usize capacity cast to f64 for token tracking — acceptable for rate limiting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize capacity cast to f64 for token tracking — acceptable for rate limiting"
    )]
    pub fn new(capacity: usize, refill_rate: f64) -> Result<Self, crate::ConfigError> {
        if capacity == 0 || capacity > 100_000 {
            return Err(crate::ConfigError::new("capacity", "must be 1..=100,000"));
        }
        if !(0.001..=10_000.0).contains(&refill_rate) {
            return Err(crate::ConfigError::new(
                "refill_rate",
                "must be 0.001..=10,000.0",
            ));
        }
        Ok(Self {
            capacity,
            state: Mutex::new(TokenBucketState {
                tokens: capacity as f64,
                last_refill: Instant::now(),
            }),
            refill_rate: AtomicU64::new(refill_rate.to_bits()),
            burst_size: AtomicUsize::new(capacity),
        })
    }

    /// Set burst size (clamped to `1..=100,000`).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_burst(self, burst_size: usize) -> Self {
        self.burst_size
            .store(burst_size.clamp(1, 100_000), Ordering::Release);
        self
    }

    /// Update the refill rate in-place, avoiding a re-allocation.
    ///
    /// The new rate is applied on the next `acquire()` call.
    /// `new_rate` is clamped to the same range accepted by `new()`.
    pub fn update_rate(&self, new_rate: f64) {
        let clamped = new_rate.clamp(0.001, 10_000.0);
        self.refill_rate.store(clamped.to_bits(), Ordering::Release);
    }

    /// Update the burst size in-place.
    ///
    /// Used by the adaptive rate limiter to keep burst capacity in sync with the
    /// adjusted rate. Clamped to `1..=100,000`.
    pub fn update_burst(&self, new_burst: usize) {
        self.burst_size
            .store(new_burst.clamp(1, 100_000), Ordering::Release);
    }
}

impl RateLimiter for TokenBucket {
    // Reason: usize burst_size cast to f64 for token math — acceptable for rate limiting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize burst_size cast to f64 for token math — acceptable for rate limiting"
    )]
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let mut state = self.state.lock();

        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let refill_rate = f64::from_bits(self.refill_rate.load(Ordering::Acquire));
        let burst = self.burst_size.load(Ordering::Acquire);
        let tokens_to_add = elapsed * refill_rate;
        state.tokens = (state.tokens + tokens_to_add).min(burst as f64);
        state.last_refill = now;

        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            drop(state);
            Ok(())
        } else {
            drop(state);
            Err(CallError::rate_limited())
        }
    }

    // Reason: usize burst_size cast to f64 for token math — acceptable for rate limiting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize burst_size cast to f64 for token math — acceptable for rate limiting"
    )]
    async fn current_rate(&self) -> f64 {
        let state = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let tokens = state.tokens;
        drop(state);
        let refill_rate = f64::from_bits(self.refill_rate.load(Ordering::Acquire));
        let burst = self.burst_size.load(Ordering::Acquire);
        elapsed.mul_add(refill_rate, tokens).min(burst as f64)
    }

    // Reason: usize burst_size cast to f64 for token reset — acceptable for rate limiting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize burst_size cast to f64 for token reset — acceptable for rate limiting"
    )]
    async fn reset(&self) {
        let mut state = self.state.lock();
        state.tokens = self.burst_size.load(Ordering::Acquire) as f64;
        state.last_refill = Instant::now();
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// LEAKY BUCKET
// ═══════════════════════════════════════════════════════════════════════════════

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

impl fmt::Debug for LeakyBucket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeakyBucket")
            .field("capacity", &self.capacity)
            .field("leak_rate", &self.leak_rate)
            .finish_non_exhaustive()
    }
}

impl LeakyBucket {
    /// Create new leaky bucket.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `capacity` is 0 or > 100,000,
    /// or `leak_rate` is outside 0.001..=10,000.0.
    pub fn new(capacity: usize, leak_rate: f64) -> Result<Self, crate::ConfigError> {
        if capacity == 0 || capacity > 100_000 {
            return Err(crate::ConfigError::new("capacity", "must be 1..=100,000"));
        }
        if !(0.001..=10_000.0).contains(&leak_rate) {
            return Err(crate::ConfigError::new(
                "leak_rate",
                "must be 0.001..=10,000.0",
            ));
        }
        Ok(Self {
            capacity,
            state: Mutex::new(LeakyBucketState {
                level: 0,
                last_leak: Instant::now(),
            }),
            leak_rate,
        })
    }
}

impl RateLimiter for LeakyBucket {
    // Reason: f64 leak amount cast to usize — acceptable for bucket level calculation.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
            Err(CallError::rate_limited())
        }
    }

    // Reason: f64 leak amount cast to usize and usize capacity cast to f64 — acceptable for rate
    // reporting.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    async fn current_rate(&self) -> f64 {
        let state = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_leak).as_secs_f64();
        let level = state.level;
        drop(state);
        let leaked = (elapsed * self.leak_rate) as usize;
        let current_level = level.saturating_sub(leaked);
        (self.capacity - current_level) as f64
    }

    async fn reset(&self) {
        let mut state = self.state.lock();
        state.level = 0;
        state.last_leak = Instant::now();
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SLIDING WINDOW
// ═══════════════════════════════════════════════════════════════════════════════

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

impl fmt::Debug for SlidingWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlidingWindow")
            .field("window_duration", &self.window_duration)
            .field("max_requests", &self.max_requests)
            .finish_non_exhaustive()
    }
}

impl SlidingWindow {
    /// Create new sliding window rate limiter.
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
            if front < cutoff {
                requests.pop_front();
            } else {
                break;
            }
        }
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
            drop(requests);
            Err(CallError::rate_limited())
        }
    }

    // Reason: usize request count cast to f64 — acceptable for rate reporting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize request count cast to f64 — acceptable for rate reporting"
    )]
    async fn current_rate(&self) -> f64 {
        let now = Instant::now();
        let mut requests = self.requests.lock();
        // Always do a full cleanup here so the reported count is accurate.
        let cutoff = now.checked_sub(self.window_duration).unwrap_or(now);
        Self::clean_old_requests_locked(&mut requests, cutoff);
        let len = requests.len() as f64;
        drop(requests);
        len
    }

    async fn reset(&self) {
        let mut requests = self.requests.lock();
        requests.clear();
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ADAPTIVE RATE LIMITER
// ═══════════════════════════════════════════════════════════════════════════════

/// Mutable state behind a single lock — only fields that need coordinated mutation.
struct AdaptiveState {
    inner: Arc<TokenBucket>,
    last_stats_reset: Instant,
    current_rate: f64,
    initial_rate: f64,
}

/// Adaptive rate limiter that adjusts based on error rates.
///
/// Automatically adjusts rate limiting based on success/error ratios.
/// - High error rate (>10%) → decrease rate
/// - Low error rate (<1%) → increase rate
///
/// Counters are lock-free atomics; the write lock is only taken for rate adjustment.
pub struct AdaptiveRateLimiter {
    state: Arc<RwLock<AdaptiveState>>,
    /// Lock-free copy of `current_rate` for cheap reads without taking the lock.
    /// Stored as `f64::to_bits()` / read via `f64::from_bits()`.
    atomic_rate: AtomicU64,
    /// Lock-free success counter — swapped to zero on adjustment.
    success_count: AtomicU64,
    /// Lock-free error counter — swapped to zero on adjustment.
    error_count: AtomicU64,
    stats_window: Duration,
    min_rate: f64,
    max_rate: f64,
}

impl fmt::Debug for AdaptiveRateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdaptiveRateLimiter")
            .field("min_rate", &self.min_rate)
            .field("max_rate", &self.max_rate)
            .field(
                "current_rate",
                &f64::from_bits(self.atomic_rate.load(Ordering::Relaxed)),
            )
            .finish_non_exhaustive()
    }
}

impl AdaptiveRateLimiter {
    /// Create new adaptive rate limiter.
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if any rate is outside `TokenBucket`'s valid range
    /// (`refill_rate` 0.001..=10,000, derived capacity 1..=100,000), if
    /// `min_rate > max_rate`, or if `initial_rate` is outside `[min_rate, max_rate]`.
    // Reason: f64 rates cast to usize for token bucket capacity — acceptable for rate limiting.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn new(
        initial_rate: f64,
        min_rate: f64,
        max_rate: f64,
    ) -> Result<Self, crate::ConfigError> {
        if min_rate > max_rate {
            return Err(crate::ConfigError::new("min_rate", "must be <= max_rate"));
        }
        if initial_rate < min_rate || initial_rate > max_rate {
            return Err(crate::ConfigError::new(
                "initial_rate",
                format!("must be within [{min_rate}, {max_rate}]"),
            ));
        }
        // Validate that all rates in [min_rate, max_rate] produce valid TokenBucket configs.
        // The extremes are sufficient since TokenBucket validates capacity and refill_rate.
        TokenBucket::new(min_rate.max(1.0) as usize, min_rate).map_err(|e| {
            crate::ConfigError::new("min_rate", format!("produces invalid TokenBucket: {e}"))
        })?;
        TokenBucket::new(max_rate.max(1.0) as usize, max_rate).map_err(|e| {
            crate::ConfigError::new("max_rate", format!("produces invalid TokenBucket: {e}"))
        })?;
        let token_bucket =
            TokenBucket::new(initial_rate.max(1.0) as usize, initial_rate).map_err(|e| {
                crate::ConfigError::new(
                    "initial_rate",
                    format!("produces invalid TokenBucket: {e}"),
                )
            })?;

        Ok(Self {
            state: Arc::new(RwLock::new(AdaptiveState {
                inner: Arc::new(token_bucket),
                last_stats_reset: Instant::now(),
                current_rate: initial_rate,
                initial_rate,
            })),
            atomic_rate: AtomicU64::new(initial_rate.to_bits()),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            stats_window: Duration::from_mins(1),
            min_rate,
            max_rate,
        })
    }

    /// Try to adjust rate if stats window has elapsed.
    ///
    /// Uses a read lock for the fast path (window not yet elapsed) and only takes
    /// a write lock when adjustment is needed.
    fn maybe_adjust_rate(&self) {
        // Fast path: read lock to check window
        let needs_adjust = {
            let state = self.state.read();
            state.last_stats_reset.elapsed() >= self.stats_window
        };

        if !needs_adjust {
            return;
        }

        // Slow path: write lock for adjustment
        let mut state = self.state.write();
        // Double-check after acquiring write lock (another thread may have adjusted)
        if state.last_stats_reset.elapsed() < self.stats_window {
            return;
        }

        let success = self.success_count.swap(0, Ordering::Relaxed);
        let error = self.error_count.swap(0, Ordering::Relaxed);
        self.do_adjust_rate(&mut state, success, error);
        drop(state);
    }

    /// Perform the rate adjustment. Caller must hold the write lock.
    // Reason: u64 counts cast to f64 for rate calculation, and f64 rate cast to usize for
    // token bucket capacity — acceptable for approximate rate limiting.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn do_adjust_rate(&self, state: &mut AdaptiveState, success: u64, error: u64) {
        let total = success + error;
        if total > 0 {
            let error_rate = error as f64 / total as f64;

            if error_rate > 0.1 {
                state.current_rate = (state.current_rate * 0.9).max(self.min_rate);
            } else if error_rate < 0.01 {
                state.current_rate = (state.current_rate * 1.1).min(self.max_rate);
            }

            // Update rate and burst capacity in-place to stay in sync.
            state.inner.update_rate(state.current_rate);
            state
                .inner
                .update_burst(state.current_rate.max(1.0) as usize);
            self.atomic_rate
                .store(state.current_rate.to_bits(), Ordering::Release);
        }

        state.last_stats_reset = Instant::now();
    }

    /// Record a successful operation.
    pub fn record_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
        self.maybe_adjust_rate();
    }

    /// Record a failed operation.
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
        self.maybe_adjust_rate();
    }
}

impl RateLimiter for AdaptiveRateLimiter {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let limiter = {
            let state = self.state.read();
            state.inner.clone()
        };

        limiter.acquire().await
    }

    async fn call<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send,
    {
        self.acquire()
            .await
            .map_err(|_| CallError::rate_limited())?;
        let result = operation().await;

        match &result {
            Ok(_) => self.record_success(),
            Err(_) => self.record_error(),
        }

        result.map_err(CallError::Operation)
    }

    async fn current_rate(&self) -> f64 {
        f64::from_bits(self.atomic_rate.load(Ordering::Acquire))
    }

    // Reason: f64 rate cast to usize for token bucket capacity — acceptable for rate limiting.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    async fn reset(&self) {
        self.success_count.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        let mut state = self.state.write();
        state.last_stats_reset = Instant::now();
        let mut reset_rate = state.initial_rate.clamp(0.001, 10_000.0);
        let reset_capacity = reset_rate.max(1.0) as usize;

        let new_bucket = if let Ok(bucket) = TokenBucket::new(reset_capacity, reset_rate) {
            bucket
        } else {
            // Safe fallback for release builds if invariants ever drift.
            debug_assert!(false, "initial_rate should always reconstruct TokenBucket");
            reset_rate = 1.0;
            if let Ok(bucket) = TokenBucket::new(1, reset_rate) {
                bucket
            } else {
                return;
            }
        };

        state.current_rate = reset_rate;
        state.inner = Arc::new(new_bucket);
        drop(state);
        self.atomic_rate
            .store(reset_rate.to_bits(), Ordering::Release);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// GOVERNOR RATE LIMITER
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "governor")]
mod governor_impl {
    use std::{fmt, num::NonZeroU32, time::Duration};

    use governor::{DefaultDirectRateLimiter, Quota, RateLimiter as GovernorLimiter};

    use super::RateLimiter;
    use crate::CallError;

    /// Governor-based rate limiter using GCRA (Generic Cell Rate Algorithm)
    ///
    /// Production-grade, sub-millisecond precision, lock-free implementation.
    pub struct GovernorRateLimiter {
        limiter: DefaultDirectRateLimiter,
        rate_per_second: f64,
        burst_capacity: u32,
    }

    impl fmt::Debug for GovernorRateLimiter {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("GovernorRateLimiter")
                .field("rate_per_second", &self.rate_per_second)
                .field("burst_capacity", &self.burst_capacity)
                .finish_non_exhaustive()
        }
    }

    impl GovernorRateLimiter {
        /// Create a new governor-based rate limiter.
        #[must_use]
        pub fn new(rate_per_second: f64, burst_capacity: u32) -> Self {
            let safe_rate = if rate_per_second.is_finite() {
                rate_per_second.clamp(0.001, 1_000_000.0)
            } else {
                0.001
            };
            let safe_burst = burst_capacity.min(100_000);
            let burst = NonZeroU32::new(safe_burst.max(1)).unwrap_or(NonZeroU32::MIN);

            let request_period =
                Duration::from_secs_f64(1.0 / safe_rate).max(Duration::from_nanos(1));
            let quota = Quota::with_period(request_period).map_or_else(
                || Quota::per_second(NonZeroU32::MIN).allow_burst(burst),
                |base| base.allow_burst(burst),
            );

            Self {
                limiter: GovernorLimiter::direct(quota),
                rate_per_second: safe_rate,
                burst_capacity: safe_burst,
            }
        }

        /// Returns the configured rate per second.
        #[must_use]
        pub const fn rate_per_second(&self) -> f64 {
            self.rate_per_second
        }

        /// Returns the configured burst capacity.
        #[must_use]
        pub const fn burst_capacity(&self) -> u32 {
            self.burst_capacity
        }

        /// Create with custom quota for advanced use cases.
        #[must_use]
        pub fn with_quota(quota: Quota) -> Self {
            Self {
                limiter: GovernorLimiter::direct(quota),
                rate_per_second: 0.0,
                burst_capacity: 0,
            }
        }
    }

    impl RateLimiter for GovernorRateLimiter {
        async fn acquire(&self) -> Result<(), CallError<()>> {
            match self.limiter.check() {
                Ok(()) => Ok(()),
                Err(_) => Err(CallError::rate_limited()),
            }
        }

        async fn current_rate(&self) -> f64 {
            self.rate_per_second
        }

        async fn reset(&self) {
            // GCRA state decays naturally — no-op for trait compatibility
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn rate_limited_after_burst_exhausted() {
            let limiter = GovernorRateLimiter::new(10.0, 5);

            for _ in 0..5 {
                assert!(limiter.acquire().await.is_ok());
            }

            let result = limiter.acquire().await;
            assert!(matches!(result, Err(CallError::RateLimited { .. })));
        }

        #[tokio::test]
        async fn call_succeeds_within_capacity() {
            let limiter = GovernorRateLimiter::new(100.0, 10);
            let result = limiter.call(|| async { Ok::<i32, &str>(42) }).await;
            assert_eq!(result.unwrap(), 42);
        }

        #[tokio::test]
        async fn non_finite_rate_is_safely_clamped() {
            let limiter = GovernorRateLimiter::new(f64::NAN, 5);
            let rate = limiter.current_rate().await;

            assert!(rate.is_finite());
            assert!((rate - 0.001).abs() < f64::EPSILON);
        }
    }
}

#[cfg(feature = "governor")]
pub use governor_impl::GovernorRateLimiter;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_bucket_respects_capacity() {
        let limiter = TokenBucket::new(1, 0.001).unwrap();
        assert!(limiter.acquire().await.is_ok());
        assert!(limiter.acquire().await.is_err());
    }

    #[test]
    fn leaky_bucket_rejects_zero_capacity() {
        assert!(LeakyBucket::new(0, 1.0).is_err());
    }

    #[test]
    fn leaky_bucket_rejects_invalid_leak_rate() {
        assert!(LeakyBucket::new(10, 0.0).is_err());
        assert!(LeakyBucket::new(10, -1.0).is_err());
    }

    #[test]
    fn leaky_bucket_accepts_valid_config() {
        assert!(LeakyBucket::new(10, 1.0).is_ok());
    }

    #[test]
    fn sliding_window_rejects_zero_requests() {
        assert!(SlidingWindow::new(Duration::from_secs(1), 0).is_err());
    }

    #[test]
    fn sliding_window_rejects_zero_duration() {
        assert!(SlidingWindow::new(Duration::ZERO, 10).is_err());
    }

    #[test]
    fn sliding_window_accepts_valid_config() {
        assert!(SlidingWindow::new(Duration::from_secs(1), 10).is_ok());
    }

    // ── B2: AdaptiveRateLimiter rejects initial_rate outside bounds ──────

    #[test]
    fn adaptive_rejects_initial_rate_below_min() {
        let result = AdaptiveRateLimiter::new(1.0, 10.0, 100.0);
        assert!(result.is_err(), "should reject initial_rate below min_rate");
    }

    #[test]
    fn adaptive_rejects_initial_rate_above_max() {
        let result = AdaptiveRateLimiter::new(500.0, 10.0, 100.0);
        assert!(result.is_err(), "should reject initial_rate above max_rate");
    }

    #[test]
    fn adaptive_accepts_initial_rate_at_bounds() {
        assert!(AdaptiveRateLimiter::new(10.0, 10.0, 100.0).is_ok());
        assert!(AdaptiveRateLimiter::new(100.0, 10.0, 100.0).is_ok());
    }

    // ── B1: update_burst keeps burst in sync with rate ──────────────────

    #[tokio::test]
    async fn token_bucket_update_burst_limits_tokens() {
        let limiter = TokenBucket::new(10, 0.001).unwrap();
        // Exhaust initial tokens
        for _ in 0..10 {
            assert!(limiter.acquire().await.is_ok());
        }
        assert!(limiter.acquire().await.is_err());

        // Reset and reduce burst to 3
        limiter.reset().await;
        limiter.update_burst(3);

        // Should only get 3 tokens now (burst caps refill)
        for _ in 0..3 {
            assert!(limiter.acquire().await.is_ok());
        }
        assert!(limiter.acquire().await.is_err());
    }

    // ── M3: atomic counters work correctly ──────────────────────────────

    #[tokio::test]
    async fn adaptive_record_success_and_error_are_lock_free() {
        let limiter = AdaptiveRateLimiter::new(50.0, 10.0, 100.0).unwrap();
        // Should not deadlock or panic with many concurrent calls
        for _ in 0..100 {
            limiter.record_success();
        }
        for _ in 0..50 {
            limiter.record_error();
        }
        // Rate should still be around initial since stats_window (1 min) hasn't elapsed
        let rate = limiter.current_rate().await;
        assert!((rate - 50.0).abs() < 0.001, "expected ~50.0, got {rate}");
    }
}
