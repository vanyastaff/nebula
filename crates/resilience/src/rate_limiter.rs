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
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::{Mutex, RwLock};

use crate::{CallContext, CallError};

fn retry_after_from_rate(units_needed: f64, units_per_second: f64) -> Option<Duration> {
    if !units_needed.is_finite() || !units_per_second.is_finite() || units_per_second <= 0.0 {
        return None;
    }
    if units_needed <= 0.0 {
        return Some(Duration::ZERO);
    }

    let seconds = units_needed / units_per_second;
    if !seconds.is_finite() {
        return None;
    }

    Some(Duration::from_secs_f64(seconds).max(Duration::from_nanos(1)))
}

fn duration_as_nanos_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn rate_limited_with_retry_after(retry_after: Option<Duration>) -> CallError<()> {
    retry_after.map_or_else(CallError::rate_limited, CallError::rate_limited_after)
}

pub(crate) fn map_acquire_error<E>(err: CallError<()>) -> CallError<E> {
    match err {
        CallError::RateLimited { retry_after } => CallError::RateLimited { retry_after },
        CallError::Timeout(duration) => CallError::Timeout(duration),
        CallError::CircuitOpen => CallError::CircuitOpen,
        CallError::BulkheadFull => CallError::BulkheadFull,
        CallError::Cancelled { reason } => CallError::Cancelled { reason },
        CallError::TaskPanicked => CallError::TaskPanicked,
        CallError::LoadShed => CallError::LoadShed,
        CallError::FallbackFailed { reason } => CallError::FallbackFailed { reason },
        CallError::FallbackFailedWithContext { primary, fallback } => {
            CallError::FallbackFailedWithContext {
                primary: Box::new(map_acquire_error(*primary)),
                fallback: Box::new(map_acquire_error(*fallback)),
            }
        },
        CallError::Operation(()) | CallError::RetriesExhausted { .. } => CallError::rate_limited(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// STATUS
// ═══════════════════════════════════════════════════════════════════════════════

/// Current state of a rate limiter, uniform across algorithms.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::{RateLimiter, rate_limiter::TokenBucket};
///
/// # #[tokio::main]
/// # async fn main() {
/// let limiter = TokenBucket::new(10, 1.0).expect("valid config");
/// let status = limiter.status().await;
/// assert!(status.remaining <= 10.0);
/// assert_eq!(status.limit_per_second, Some(1.0));
/// # }
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct RateLimiterStatus {
    /// Permits available right now, in the limiter's own permit units.
    ///
    /// This is the number of immediately successful `acquire()` calls
    /// (fractional when the bucket refills continuously). It is never a
    /// rate: `SlidingWindow` reports its remaining window quota here.
    pub remaining: f64,
    /// Configured steady-state rate in permits per second, when the
    /// algorithm has one.
    ///
    /// `TokenBucket` and `LeakyBucket` fill this from their refill/leak rate;
    /// `AdaptiveRateLimiter` from its current tuned rate. `SlidingWindow`
    /// returns `None` — it enforces a count per window, not a rate.
    pub limit_per_second: Option<f64>,
}

impl RateLimiterStatus {
    /// Build a status from a remaining-quota count and an optional rate.
    #[must_use]
    pub const fn new(remaining: f64, limit_per_second: Option<f64>) -> Self {
        Self {
            remaining,
            limit_per_second,
        }
    }
}

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
/// # Why `acquire` is `async`
///
/// A limiter that enforces a strict outflow rate must be able to park a caller
/// until a permit is free; a synchronous `Result` signature would force such
/// limiters to spin or lie. The built-in limiters all fail fast instead (they
/// return `RateLimited` immediately), so their `async` bodies complete without
/// awaiting — that is an implementation choice, not the contract. Implementors
/// that queue callers must bound the wait themselves.
///
/// Because `impl Future` in return position makes the trait not object-safe,
/// [`ErasedRateLimiter`] is the object-safe facade for heterogeneous registries.
///
/// # Implementing for third-party types
///
/// This trait is `sealed`-free and designed for downstream implementation.
/// New methods will always provide default implementations to avoid breaking
/// changes across minor versions.
///
/// [`TokenBucket`] and [`SlidingWindow`] are complete implementations to copy
/// from; the [module documentation](self) shows both in use.
pub trait RateLimiter: Send + Sync {
    /// Attempt to consume one permit from the rate limiter.
    ///
    /// Returns `Ok(())` when the request is allowed to proceed, or
    /// <code>Err([`CallError::RateLimited`])</code> when the current rate would be
    /// exceeded. The error may include a `retry_after` hint for callers
    /// that want to back off before retrying.
    fn acquire(&self) -> impl Future<Output = Result<(), CallError<()>>> + Send;

    /// Attempt to consume one permit with cancellation/deadline from a shared
    /// policy context.
    ///
    /// Implementations do not need to override this unless they can enforce the
    /// context more efficiently internally.
    fn acquire_with_context<'a>(
        &'a self,
        context: &'a CallContext,
    ) -> impl Future<Output = Result<(), CallError<()>>> + Send + 'a {
        async move { context.run_result(self.acquire()).await }
    }

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
            self.acquire().await.map_err(map_acquire_error)?;
            operation().await.map_err(CallError::Operation)
        }
    }

    /// Acquire a permit using `context`, then execute `operation` under the
    /// same cancellation/deadline contract.
    ///
    /// The operation is not invoked when the limiter rejects, the context is
    /// cancelled, or the context deadline expires before a permit is acquired.
    fn call_with_context<'a, T, E, F, Fut>(
        &'a self,
        context: &'a CallContext,
        operation: F,
    ) -> impl Future<Output = Result<T, CallError<E>>> + Send + 'a
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
        T: Send + 'a,
        E: Send + 'a,
    {
        async move {
            self.acquire_with_context(context)
                .await
                .map_err(map_acquire_error)?;
            context
                .run_result(async { operation().await.map_err(CallError::Operation) })
                .await
        }
    }

    /// Report the limiter's current state.
    ///
    /// [`remaining`](RateLimiterStatus::remaining) answers "would `acquire()`
    /// succeed right now, and how many times?" for every implementation.
    /// [`limit_per_second`](RateLimiterStatus::limit_per_second) is the
    /// configured steady-state rate where the algorithm has one
    /// (`None` for window counters, whose whole point is the window, not a
    /// rate).
    ///
    /// The field names follow the IETF `RateLimit` header vocabulary
    /// (`RateLimit-Remaining` / `RateLimit-Limit`): a status reports quota,
    /// not an algorithm-specific internal counter.
    fn status(&self) -> impl Future<Output = RateLimiterStatus> + Send;

    /// Clears all state and resets to initial conditions.
    fn reset(&self) -> impl Future<Output = ()> + Send;
}

type BoxRateLimiterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe facade for storing heterogeneous rate limiters in registries.
///
/// [`RateLimiter`] keeps `impl Future` methods for static dispatch and zero
/// allocation in direct calls, which makes that trait intentionally not
/// object-safe. Use `Arc<dyn ErasedRateLimiter>` when a tenant/resource registry
/// needs to store multiple limiter implementations behind one type.
///
/// **No registry in this workspace stores `dyn ErasedRateLimiter` yet.** It is
/// the seam a host uses when limiter selection is dynamic (tenant- or
/// resource-scoped policies); the engine currently builds one concrete
/// `TokenBucket` per action key and does not need the facade. It is kept
/// because removing object safety from `RateLimiter` later would be a breaking
/// change, not because there is a current consumer.
///
/// The facade exposes only non-generic operations. To run an operation through a
/// limiter, acquire through this trait and then call the operation yourself, or
/// pass the object to
/// [`PipelineBuilder::rate_limiter_erased`](crate::PipelineBuilder::rate_limiter_erased).
///
/// The [module documentation](self) shows a `Vec<Arc<dyn ErasedRateLimiter>>`
/// registry.
pub trait ErasedRateLimiter: Send + Sync {
    /// Attempt to consume one permit from the rate limiter.
    fn acquire_boxed(&self) -> BoxRateLimiterFuture<'_, Result<(), CallError<()>>>;

    /// Attempt to consume one permit with cancellation/deadline from a shared
    /// policy context.
    fn acquire_with_context_boxed<'a>(
        &'a self,
        context: &'a CallContext,
    ) -> BoxRateLimiterFuture<'a, Result<(), CallError<()>>> {
        Box::pin(context.run_result(self.acquire_boxed()))
    }

    /// Report the limiter's current state (see [`RateLimiter::status`]).
    fn status_boxed(&self) -> BoxRateLimiterFuture<'_, RateLimiterStatus>;

    /// Clears all state and resets to initial conditions.
    fn reset_boxed(&self) -> BoxRateLimiterFuture<'_, ()>;
}

impl<T> ErasedRateLimiter for T
where
    T: RateLimiter,
{
    fn acquire_boxed(&self) -> BoxRateLimiterFuture<'_, Result<(), CallError<()>>> {
        Box::pin(self.acquire())
    }

    fn acquire_with_context_boxed<'a>(
        &'a self,
        context: &'a CallContext,
    ) -> BoxRateLimiterFuture<'a, Result<(), CallError<()>>> {
        Box::pin(self.acquire_with_context(context))
    }

    fn status_boxed(&self) -> BoxRateLimiterFuture<'_, RateLimiterStatus> {
        Box::pin(self.status())
    }

    fn reset_boxed(&self) -> BoxRateLimiterFuture<'_, ()> {
        Box::pin(self.reset())
    }
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

    /// Updates the refill rate in-place, avoiding a re-allocation.
    ///
    /// The new rate is applied on the next `acquire()` call.
    /// `new_rate` is clamped to the same range accepted by `new()`.
    pub fn update_rate(&self, new_rate: f64) {
        let clamped = if new_rate.is_finite() {
            new_rate.clamp(0.001, 10_000.0)
        } else {
            0.001
        };
        self.refill_rate.store(clamped.to_bits(), Ordering::Release);
    }

    /// Updates the burst size in-place.
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
            let retry_after = retry_after_from_rate(1.0 - state.tokens, refill_rate);
            drop(state);
            Err(rate_limited_with_retry_after(retry_after))
        }
    }

    // Reason: usize burst_size cast to f64 for token math — acceptable for rate limiting.
    #[expect(
        clippy::cast_precision_loss,
        reason = "usize burst_size cast to f64 for token math — acceptable for rate limiting"
    )]
    async fn status(&self) -> RateLimiterStatus {
        let state = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let tokens = state.tokens;
        drop(state);
        let refill_rate = f64::from_bits(self.refill_rate.load(Ordering::Acquire));
        let burst = self.burst_size.load(Ordering::Acquire);
        RateLimiterStatus::new(
            elapsed.mul_add(refill_rate, tokens).min(burst as f64),
            Some(refill_rate),
        )
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
    /// Creates a new leaky bucket.
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

    // Reason: f64/usize conversions are acceptable for approximate leak accounting.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn leak_locked(state: &mut LeakyBucketState, leak_rate: f64, now: Instant) {
        if state.level == 0 {
            state.last_leak = now;
            return;
        }

        let elapsed = now.duration_since(state.last_leak).as_secs_f64();
        let leaked = (elapsed * leak_rate) as usize;

        if leaked == 0 {
            return;
        }

        if leaked >= state.level {
            state.level = 0;
            state.last_leak = now;
            return;
        }

        state.level -= leaked;
        let leaked_secs = leaked as f64 / leak_rate;
        let drain_duration = Duration::from_secs_f64(leaked_secs);
        state.last_leak = state.last_leak.checked_add(drain_duration).unwrap_or(now);
    }

    fn retry_after_locked(
        state: &LeakyBucketState,
        leak_rate: f64,
        now: Instant,
    ) -> Option<Duration> {
        let elapsed = now.duration_since(state.last_leak).as_secs_f64();
        let units_until_next_leak = elapsed.mul_add(-leak_rate, 1.0).max(0.0);
        retry_after_from_rate(units_until_next_leak, leak_rate)
    }
}

impl RateLimiter for LeakyBucket {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        let mut state = self.state.lock();
        let now = Instant::now();
        Self::leak_locked(&mut state, self.leak_rate, now);

        if state.level < self.capacity {
            if state.level == 0 {
                state.last_leak = now;
            }
            state.level += 1;
            drop(state);
            Ok(())
        } else {
            let retry_after = Self::retry_after_locked(&state, self.leak_rate, now);
            drop(state);
            Err(rate_limited_with_retry_after(retry_after))
        }
    }

    // Reason: f64 leak amount cast to usize and usize capacity cast to f64 — acceptable for rate
    // reporting.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    async fn status(&self) -> RateLimiterStatus {
        let state = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_leak).as_secs_f64();
        let level = state.level;
        drop(state);
        let leaked = (elapsed * self.leak_rate) as usize;
        let current_level = level.saturating_sub(leaked);
        RateLimiterStatus::new((self.capacity - current_level) as f64, Some(self.leak_rate))
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
/// Lock-free atomics are used for the counters and fast-path window check, so
/// recording outcomes does not take the rate-adjustment lock until the window
/// elapses.
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
    adjustment_origin: Instant,
    next_adjust_after_ns: AtomicU64,
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

/// Re-arms the adaptive limiter's next-adjustment deadline on drop.
///
/// Exists so the `u64::MAX` adjustment sentinel cannot survive an unwind in
/// the write section; see [`AdaptiveRateLimiter::maybe_adjust_rate`].
struct RearmOnDrop<'a> {
    limiter: &'a AdaptiveRateLimiter,
}

impl Drop for RearmOnDrop<'_> {
    fn drop(&mut self) {
        let elapsed_ns = duration_as_nanos_u64(self.limiter.adjustment_origin.elapsed());
        let window_ns = duration_as_nanos_u64(self.limiter.stats_window);
        self.limiter
            .next_adjust_after_ns
            .store(elapsed_ns.saturating_add(window_ns), Ordering::Release);
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
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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

        let now = Instant::now();
        let stats_window = Duration::from_mins(1);
        Ok(Self {
            state: Arc::new(RwLock::new(AdaptiveState {
                inner: Arc::new(token_bucket),
                last_stats_reset: now,
                current_rate: initial_rate,
                initial_rate,
            })),
            adjustment_origin: now,
            next_adjust_after_ns: AtomicU64::new(duration_as_nanos_u64(stats_window)),
            atomic_rate: AtomicU64::new(initial_rate.to_bits()),
            success_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            stats_window,
            min_rate,
            max_rate,
        })
    }

    /// Try to adjust rate if stats window has elapsed.
    ///
    /// Uses an atomic deadline for the fast path (window not yet elapsed) and
    /// only takes a write lock when adjustment is needed.
    fn maybe_adjust_rate(&self) {
        let elapsed_ns = duration_as_nanos_u64(self.adjustment_origin.elapsed());
        let next_adjust_after_ns = self.next_adjust_after_ns.load(Ordering::Acquire);
        if elapsed_ns < next_adjust_after_ns {
            return;
        }

        if self
            .next_adjust_after_ns
            .compare_exchange(
                next_adjust_after_ns,
                u64::MAX,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return;
        }

        // `u64::MAX` above is the adjustment lock. It must be cleared on every
        // exit from this slow path — including an unwind inside the write
        // section — because a leaked `u64::MAX` permanently disables
        // self-tuning: every later call reads `elapsed < next_adjust_after`
        // and returns. The RAII guard is what re-arms the window.
        let rearm = RearmOnDrop { limiter: self };

        // Slow path: write lock for adjustment
        let mut state = self.state.write();
        // Double-check after acquiring write lock (another thread may have adjusted)
        if state.last_stats_reset.elapsed() >= self.stats_window {
            let success = self.success_count.swap(0, Ordering::Relaxed);
            let error = self.error_count.swap(0, Ordering::Relaxed);
            self.do_adjust_rate(&mut state, success, error);
        }
        drop(state);

        drop(rearm);
    }

    /// Perform the rate adjustment. Caller must hold the write lock.
    // Reason: u64 counts cast to f64 for rate calculation, and f64 rate cast to usize for
    // token bucket capacity — acceptable for approximate rate limiting.
    #[expect(
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

    /// Records a successful operation.
    pub fn record_success(&self) {
        self.success_count.fetch_add(1, Ordering::Relaxed);
        self.maybe_adjust_rate();
    }

    /// Records a failed operation.
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
        self.acquire().await.map_err(map_acquire_error)?;
        let result = operation().await;

        match &result {
            Ok(_) => self.record_success(),
            Err(_) => self.record_error(),
        }

        result.map_err(CallError::Operation)
    }

    async fn call_with_context<'a, T, E, F, Fut>(
        &'a self,
        context: &'a CallContext,
        operation: F,
    ) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send + 'a,
        Fut: Future<Output = Result<T, E>> + Send + 'a,
        T: Send + 'a,
        E: Send + 'a,
    {
        self.acquire_with_context(context)
            .await
            .map_err(map_acquire_error)?;

        let result = context
            .run_result(async { operation().await.map_err(CallError::Operation) })
            .await;

        match &result {
            Ok(_) => self.record_success(),
            Err(_) => self.record_error(),
        }

        result
    }

    async fn status(&self) -> RateLimiterStatus {
        // The adaptive limiter's remaining quota is its inner bucket's;
        // the tuned rate is the one it reports.
        let limiter = {
            let state = self.state.read();
            state.inner.clone()
        };
        let inner = limiter.status().await;
        RateLimiterStatus::new(
            inner.remaining,
            Some(f64::from_bits(self.atomic_rate.load(Ordering::Acquire))),
        )
    }

    // Reason: f64 rate cast to usize for token bucket capacity — acceptable for rate limiting.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    async fn reset(&self) {
        self.success_count.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        let mut state = self.state.write();
        state.last_stats_reset = Instant::now();
        let elapsed_ns = duration_as_nanos_u64(self.adjustment_origin.elapsed());
        let window_ns = duration_as_nanos_u64(self.stats_window);
        self.next_adjust_after_ns
            .store(elapsed_ns.saturating_add(window_ns), Ordering::Release);
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

#[cfg(test)]
#[path = "rate_limiter_tests.rs"]
mod tests;
