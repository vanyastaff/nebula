//! Rate limiting implementations.
//!
//! This module provides the [`RateLimiter`] trait and multiple built-in algorithms:
//!
//! | Implementation | Algorithm | Best for |
//! |---|---|---|
//! | [`TokenBucket`] | Token bucket with configurable refill rate | Bursty traffic with a steady average |
//! | [`LeakyBucket`] | Leaky bucket with constant drain rate | Smoothing request bursts into a constant outflow |
//! | [`SlidingWindow`] | Sliding time-window counter | Hard per-window request caps |
//! | [`AdaptiveRateLimiter`] | Token bucket auto-tuned by error rate | Self-protecting services with variable load |
//! | `GovernorRateLimiter` | GCRA (Generic Cell Rate Algorithm) | Production, sub-millisecond precision (requires `governor` feature) |
//!
//! # Trait contract
//!
//! Every implementation must satisfy the [`RateLimiter`] contract:
//!
//! - [`acquire()`](RateLimiter::acquire) — attempt to consume one permit. Returns `Ok(())` when the
//!   request is allowed, `Err(CallError::RateLimited)` when the limit is exceeded.
//! - [`call()`](RateLimiter::call) — convenience wrapper that calls `acquire()` then executes the
//!   supplied async closure. On success the closure's return value is forwarded; on rate-limit the
//!   closure is never invoked.
//! - Implementors must be `Send + Sync` so they can be shared across tasks and stored inside
//!   `Arc<T>`.
//!
//! # Standalone usage
//!
//! All rate limiters work independently — no pipeline required:
//!
//! ```rust
//! use nebula_resilience::{RateLimiter, rate_limiter::TokenBucket};
//!
//! # #[tokio::main]
//! # async fn main() {
//! // Allow up to 100 tokens; refill at 10 tokens/second.
//! let limiter = TokenBucket::new(100, 10.0).unwrap();
//!
//! // Returns Ok(()) when a permit is available.
//! limiter.acquire().await.expect("rate limit not yet reached");
//!
//! // Or use call() to gate an operation:
//! let result = limiter
//!     .call(|| async { Ok::<&str, &str>("response") })
//!     .await;
//! assert!(result.is_ok());
//! # }
//! ```
//!
//! # Pipeline integration
//!
//! When using a rate limiter inside a [`ResiliencePipeline`](crate::ResiliencePipeline) the
//! limiter must be wrapped in [`Arc`] before being passed to
//! [`rate_limiter_from()`](crate::PipelineBuilder::rate_limiter_from):
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use nebula_resilience::{ResiliencePipeline, rate_limiter::TokenBucket};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Arc is required so the limiter can be cloned into the async closure
//! // that the pipeline builds internally — the closure must be
//! // `Send + Sync + 'static`, which demands shared ownership.
//! let rl = Arc::new(TokenBucket::new(100, 10.0)?);
//!
//! let pipeline = ResiliencePipeline::<String>::builder()
//!     .rate_limiter_from(rl)
//!     .build();
//!
//! let _result: Result<String, _> = pipeline
//!     .call(|| Box::pin(async { Ok::<_, String>("ok".into()) }))
//!     .await;
//! # Ok(())
//! # }
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

/// Core abstraction for all rate-limiting implementations.
///
/// # Contract
///
/// - [`acquire()`](RateLimiter::acquire) **must** return `Ok(())` when a permit is granted and
///   <code>Err([`CallError::RateLimited`])</code> when the rate limit is exceeded. It must never
///   block indefinitely — implementations that queue callers should enforce a timeout or queue
///   bound.
/// - [`call()`](RateLimiter::call) is a convenience wrapper over `acquire()` + operation
///   invocation. The default implementation is correct for the vast majority of cases. Override it
///   only when you need to observe the operation result (e.g., [`AdaptiveRateLimiter`] tracks
///   errors to tune its rate).
/// - **Thread safety**: all implementors must be `Send + Sync` so they can be shared across async
///   tasks via `Arc<T>`.
///
/// # Implementing for third-party types
///
/// This trait is `sealed`-free and designed for downstream implementation.
/// New methods will always provide default implementations to avoid breaking
/// changes across minor versions.
pub trait RateLimiter: Send + Sync {
    /// Attempt to consume one permit from the rate limiter.
    ///
    /// Returns `Ok(())` when the request is allowed to proceed, or
    /// <code>Err([`CallError::RateLimited`])</code> when the current rate would be
    /// exceeded. The error may include a `retry_after` hint for callers
    /// that want to back off before retrying.
    fn acquire(&self) -> impl Future<Output = Result<(), CallError<()>>> + Send;

    /// Acquire a permit and, if successful, execute `operation`.
    ///
    /// Returns <code>Err([`CallError::RateLimited`])</code> without calling `operation`
    /// when the rate limit is exceeded, or the operation's own error wrapped in
    /// [`CallError::Operation`] when `operation`
    /// itself fails.
    ///
    /// The default implementation calls [`acquire()`](Self::acquire) then
    /// invokes `operation`. Override this only when you need to observe the
    /// operation result — for example, to adjust internal counters as
    /// [`AdaptiveRateLimiter`] does.
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

/// Rate limiter based on the **token bucket** algorithm.
///
/// A bucket starts full and holds up to `capacity` tokens. Each `acquire()`
/// call consumes one token. Tokens are replenished continuously at
/// `refill_rate` tokens per second. When the bucket is empty `acquire()`
/// returns [`CallError::RateLimited`] immediately
/// — it does **not** queue or sleep.
///
/// An optional burst cap (set via [`with_burst`](Self::with_burst)) limits how
/// many tokens can accumulate during idle periods, preventing a long idle
/// period from creating a large, sudden burst.
///
/// # When to choose this
///
/// Use [`TokenBucket`] when you want to allow short bursts up to `capacity`
/// while enforcing a steady long-term average of `refill_rate` req/s. It is
/// the right default for most outbound-call rate limiting.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::rate_limiter::TokenBucket;
///
/// // Up to 100 tokens; refill at 10 per second; burst capped at 20.
/// let limiter = TokenBucket::new(100, 10.0).unwrap().with_burst(20);
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

/// Rate limiter based on the **leaky bucket** algorithm.
///
/// A virtual bucket fills up by one slot on each `acquire()` call and drains
/// ("leaks") at a constant `leak_rate` per second. When the bucket is full
/// `acquire()` returns [`CallError::RateLimited`]
/// immediately.
///
/// Unlike [`TokenBucket`], the leaky bucket enforces a strict outflow rate:
/// no matter how fast requests arrive, outgoing permit grants are smoothed
/// to the configured `leak_rate`.
///
/// # Configuration
///
/// - `capacity` — maximum bucket depth; controls the burst tolerance (how many requests can queue
///   up before being rejected).
/// - `leak_rate` — permits drained per second (0.001..=10,000).
///
/// # When to choose this
///
/// Use [`LeakyBucket`] when the downstream service requires a smooth,
/// constant request rate and cannot tolerate sudden bursts — for example,
/// a third-party API that enforces strict per-second billing quotas.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::{RateLimiter, rate_limiter::LeakyBucket};
///
/// # #[tokio::main]
/// # async fn main() {
/// // Capacity of 10 in-flight requests, draining at 5 req/s.
/// let limiter = LeakyBucket::new(10, 5.0).expect("valid config");
///
/// limiter.acquire().await.expect("first slot is free");
/// # }
/// ```
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

/// Rate limiter that **self-tunes** based on observed operation error rates.
///
/// Wraps a [`TokenBucket`] and periodically adjusts its refill rate based on
/// the ratio of successful to failed operations recorded via
/// [`record_success()`](Self::record_success) and
/// [`record_error()`](Self::record_error):
///
/// | Condition | Action |
/// |---|---|
/// | Error rate > 10 % | Decrease rate by 10 % (floor: `min_rate`) |
/// | Error rate < 1 % | Increase rate by 10 % (ceiling: `max_rate`) |
/// | 1 % ≤ error rate ≤ 10 % | No change |
///
/// Adjustments happen at most once per stats window (default: 1 minute).
/// Lock-free atomics are used for the counters so recording outcomes is
/// contention-free; the write lock is only taken when the window elapses.
///
/// The [`call()`](RateLimiter::call) override automatically records success/
/// error outcomes, so manual calls to `record_*` are only needed when you
/// invoke `acquire()` directly.
///
/// # Configuration
///
/// - `initial_rate` — starting refill rate (tokens/second).
/// - `min_rate` — lower bound for automatic rate reduction.
/// - `max_rate` — upper bound for automatic rate increase.
///
/// # When to choose this
///
/// Use [`AdaptiveRateLimiter`] when the appropriate request rate is unknown
/// upfront or changes over time — for example, calling a service that
/// returns `429 Too Many Requests` under load and you want automatic back-off
/// without manual tuning.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::{RateLimiter, rate_limiter::AdaptiveRateLimiter};
///
/// # #[tokio::main]
/// # async fn main() {
/// // Start at 50 req/s; auto-tune within [10, 100] based on observed errors.
/// let limiter = AdaptiveRateLimiter::new(50.0, 10.0, 100.0).expect("valid config");
///
/// // Using `call()` automatically records success / error outcomes for tuning.
/// let value = limiter.call(|| async { Ok::<u32, &str>(7) }).await.unwrap();
/// assert_eq!(value, 7);
/// # }
/// ```
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
