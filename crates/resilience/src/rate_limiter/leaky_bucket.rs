//! Leaky bucket rate limiter.

use std::{
    fmt,
    time::{Duration, Instant},
};

use parking_lot::Mutex;

use crate::CallError;

use super::{RateLimiter, RateLimiterStatus, rate_limited_with_retry_after, retry_after_from_rate};
#[derive(Debug)]
pub(super) struct LeakyBucketState {
    pub(super) level: usize,
    pub(super) last_leak: Instant,
}

/// Rate limiter based on the **leaky bucket** algorithm.
///
/// A virtual bucket fills up by one slot on each `acquire()` call and drains
/// ("leaks") at a constant `leak_rate` per second. When the bucket is full
/// `acquire()` returns [`CallError::RateLimited`]
/// immediately.
///
/// Unlike [`TokenBucket`](super::TokenBucket), the leaky bucket enforces a strict outflow rate:
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
    pub(super) state: Mutex<LeakyBucketState>,
    /// Leak rate per second
    pub(super) leak_rate: f64,
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
    pub(super) fn leak_locked(state: &mut LeakyBucketState, leak_rate: f64, now: Instant) {
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

    // Reason: on the default x86-64 target `mul_add` lowers to a `call fma`
    // (~30 cycles via libm) because the baseline lacks hardware FMA;
    // explicit multiply+add uses `mulsd`+`addsd`. Same rationale as
    // `retry.rs`'s jitter path.
    #[expect(
        clippy::suboptimal_flops,
        reason = "mul_add emits a slow fma call on default x86-64; explicit multiply+add is faster"
    )]
    fn retry_after_locked(
        state: &LeakyBucketState,
        leak_rate: f64,
        now: Instant,
    ) -> Option<Duration> {
        let elapsed = now.duration_since(state.last_leak).as_secs_f64();
        let units_until_next_leak = (1.0 - elapsed * leak_rate).max(0.0);
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
