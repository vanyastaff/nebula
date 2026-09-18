//! Adaptive rate limiter.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::RwLock;

use crate::{CallContext, CallError};

use super::{
    RateLimiter, RateLimiterStatus, TokenBucket, duration_as_nanos_u64, map_acquire_error,
};
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
    pub(super) success_count: AtomicU64,
    /// Lock-free error counter — swapped to zero on adjustment.
    pub(super) error_count: AtomicU64,
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
