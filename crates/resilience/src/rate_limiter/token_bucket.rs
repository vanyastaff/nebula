//! Token bucket rate limiter.

use std::{
    fmt,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::Instant,
};

use parking_lot::Mutex;

use crate::CallError;

use super::{RateLimiter, RateLimiterStatus, rate_limited_with_retry_after, retry_after_from_rate};
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
    pub(super) refill_rate: AtomicU64,
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
    // Reason: on the default x86-64 target `mul_add` lowers to a `call fma`
    // (~30 cycles via libm) because the baseline lacks hardware FMA; explicit
    // multiply+add uses `mulsd`+`addsd`. Same rationale as `retry.rs`'s jitter.
    #[expect(
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "usize burst_size to f64 for token math; mul_add emits a slow fma call on default x86-64"
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
            (elapsed * refill_rate + tokens).min(burst as f64),
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
