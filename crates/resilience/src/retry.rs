//! Retry pattern — unified API with [`Classify`](nebula_error::Classify)-aware error filtering.
//!
//! When `E` implements [`Classify`](nebula_error::Classify), retry automatically skips
//! non-retryable errors (authentication, validation, etc.) and respects
//! [`retry_hint()`](nebula_error::Classify::retry_hint) as a backoff delay floor.
//!
//! # Examples
//!
//! ```rust
//! use std::time::Duration;
//!
//! use nebula_resilience::retry::{BackoffConfig, RetryConfig, retry_with};
//!
//! # #[derive(Debug)]
//! # struct MyError;
//! # impl std::fmt::Display for MyError {
//! #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "error") }
//! # }
//! # impl std::error::Error for MyError {}
//! # impl nebula_error::Classify for MyError {
//! #     fn category(&self) -> nebula_error::ErrorCategory { nebula_error::ErrorCategory::External }
//! #     fn code(&self) -> nebula_error::ErrorCode { nebula_error::ErrorCode::new("DOC:EXAMPLE") }
//! # }
//! # #[tokio::main]
//! # async fn main() {
//! let config = RetryConfig::<MyError>::new(3)
//!     .expect("max_attempts >= 1")
//!     .backoff(BackoffConfig::Fixed(Duration::from_millis(10)));
//!
//! let value = retry_with(config, || Box::pin(async { Ok::<_, MyError>(7u32) }))
//!     .await
//!     .unwrap();
//! assert_eq!(value, 7);
//! # }
//! ```

use std::{fmt, future::Future, num::NonZeroU32, sync::Arc, time::Duration};

use smallvec::SmallVec;

use crate::{
    CallError,
    classifier::{ErrorClass, ErrorClassifier, FnClassifier},
    deadline::Deadline,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Backoff ───────────────────────────────────────────────────────────────────

/// Backoff strategy for retry delays.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::retry::BackoffConfig;
///
/// // Standard exponential: 100 ms base, 2× multiplier, capped at 30 s.
/// let exp = BackoffConfig::exponential_default();
/// assert_eq!(exp.delay_for(0), Duration::from_millis(100));
/// assert_eq!(exp.delay_for(1), Duration::from_millis(200));
///
/// // Fixed delay between every attempt.
/// let fixed = BackoffConfig::Fixed(Duration::from_millis(50));
/// assert_eq!(fixed.delay_for(5), Duration::from_millis(50));
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BackoffConfig {
    /// Same delay between every attempt.
    Fixed(Duration),
    /// Linearly increasing delay, capped at `max`.
    Linear {
        /// Base delay for the first retry.
        base: Duration,
        /// Maximum delay cap.
        max: Duration,
    },
    /// Exponentially increasing delay, capped at `max`.
    Exponential {
        /// Base delay for the first retry.
        base: Duration,
        /// Multiplier applied each attempt.
        multiplier: f64,
        /// Maximum delay cap.
        max: Duration,
    },
    /// Fibonacci-increasing delay (1, 1, 2, 3, 5, 8...), capped at `max`.
    Fibonacci {
        /// Base delay multiplied by the Fibonacci number.
        base: Duration,
        /// Maximum delay cap.
        max: Duration,
    },
    /// A user-provided sequence of delays. If attempt exceeds the list, the last delay repeats.
    ///
    /// Up to 8 delays are stored inline (no heap allocation). Larger sequences spill to the heap.
    Custom(SmallVec<[Duration; 8]>),
}

impl BackoffConfig {
    /// Standard exponential backoff: 100ms base, 2× multiplier, 30s cap.
    #[must_use]
    pub const fn exponential_default() -> Self {
        Self::Exponential {
            base: Duration::from_millis(100),
            multiplier: 2.0,
            max: Duration::from_secs(30),
        }
    }

    /// Compute the nth Fibonacci number (0-indexed: fib(0)=1, fib(1)=1, fib(2)=2, ...).
    const fn fibonacci(n: u32) -> u32 {
        let (mut a, mut b) = (1u32, 1u32);
        let mut i = 0;
        while i < n {
            let next = a.saturating_add(b);
            a = b;
            b = next;
            i += 1;
        }
        a
    }

    /// Compute the delay for the given zero-based attempt number.
    ///
    /// Useful for integrators building custom retry loops outside of [`retry_with`].
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        reason = "u128 millis cast to f64 for exponential math and u32 attempt to i32 for powi stay within configured retry bounds"
    )]
    pub fn delay_for(&self, attempt: u32) -> Duration {
        match self {
            Self::Fixed(d) => *d,
            Self::Linear { base, max } => base.saturating_mul(attempt.max(1)).min(*max),
            Self::Exponential {
                base,
                multiplier,
                max,
            } => {
                let multiplier = normalize_exponential_multiplier(*multiplier);
                if multiplier.to_bits() == 1.0f64.to_bits() {
                    return duration_from_millis_capped_u128(base.as_millis(), *max);
                }
                if multiplier.to_bits() == 2.0f64.to_bits() {
                    return exponential_delay_by_doubling(*base, attempt, *max);
                }
                let ms = base.as_millis() as f64 * multiplier.powi(attempt as i32);
                duration_from_millis_capped(ms, *max)
            },
            Self::Fibonacci { base, max } => {
                let fib_n = Self::fibonacci(attempt);
                base.saturating_mul(fib_n).min(*max)
            },
            Self::Custom(delays) => delays
                .get(attempt as usize)
                .or_else(|| delays.last())
                .copied()
                .unwrap_or(Duration::ZERO),
        }
    }
}

fn exponential_delay_by_doubling(base: Duration, attempt: u32, max: Duration) -> Duration {
    let base_ms = base.as_millis();
    let delay_ms = base_ms.checked_shl(attempt).unwrap_or(u128::MAX);
    duration_from_millis_capped_u128(delay_ms, max)
}

fn duration_from_millis_capped_u128(ms: u128, max: Duration) -> Duration {
    if ms >= max.as_millis() {
        return max;
    }

    let millis = u64::try_from(ms).unwrap_or(u64::MAX);
    Duration::from_millis(millis).min(max)
}

fn normalize_exponential_multiplier(multiplier: f64) -> f64 {
    if multiplier.is_finite() && multiplier >= 1.0 {
        multiplier
    } else {
        1.0
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded f64 millisecond math is converted back to Duration for retry delays"
)]
fn duration_from_millis_capped(ms: f64, max: Duration) -> Duration {
    if !ms.is_finite() {
        return max;
    }

    let max_ms = max.as_millis() as f64;
    if ms >= max_ms {
        return max;
    }

    Duration::from_millis(ms.max(0.0) as u64).min(max)
}

// ── JitterConfig ─────────────────────────────────────────────────────────────

/// Optional jitter to add to backoff delays.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Default)]
pub enum JitterConfig {
    /// No jitter.
    #[default]
    None,
    /// Add a random fraction up to `factor` of the delay.
    Full {
        /// Maximum jitter fraction.
        ///
        /// Values are not rejected at construction: `factor > 1.0` is capped
        /// at `1.0`, and `factor <= 0.0` or `NaN` disables jitter entirely
        /// (the base delay is returned unchanged). Keeping the builder
        /// infallible matches the other `RetryConfig` setters; the effective
        /// behavior is exactly this clamp.
        factor: f64,
        /// Optional seed for deterministic jitter (useful for testing).
        seed: Option<u64>,
    },
}

// ── RetryConfig ───────────────────────────────────────────────────────────────

/// Type alias for the on-retry notification callback.
type RetryNotify<E> = Arc<dyn Fn(&E, Duration, u32) + Send + Sync>;

/// Configuration for the retry pattern.
///
/// Error classification is driven by an optional [`ErrorClassifier`]:
/// 1. If set, [`ErrorClassifier::classify`] → [`ErrorClass::is_retryable`] decides.
/// 2. Otherwise, [`retry_with`] falls back to
///    [`Classify::is_retryable()`](nebula_error::Classify::is_retryable).
///
/// Use [`retry_if`](RetryConfig::retry_if) as shorthand for a bool-based classifier,
/// or [`with_classifier`](RetryConfig::with_classifier) for full [`ErrorClass`] control.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::retry::{BackoffConfig, JitterConfig, RetryConfig};
///
/// // Up to 5 attempts, exponential backoff, full jitter, 10 s total budget.
/// let config = RetryConfig::<&str>::new(5)
///     .expect("max_attempts >= 1")
///     .backoff(BackoffConfig::exponential_default())
///     .jitter(JitterConfig::Full {
///         factor: 0.5,
///         seed: None,
///     })
///     .total_budget(Duration::from_secs(10));
/// # let _ = config;
/// ```
pub struct RetryConfig<E = ()> {
    /// Maximum number of attempts (including the first).
    max_attempts: NonZeroU32,
    /// Backoff strategy between attempts.
    backoff: BackoffConfig,
    /// Optional jitter applied to backoff delays.
    jitter: JitterConfig,
    /// If set, retries stop when the deadline is reached. This bounds both
    /// operation execution and sleep time.
    total_budget: Option<Duration>,
    pub(crate) classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    pub(crate) on_retry: Option<RetryNotify<E>>,
    pub(crate) sink: Arc<dyn MetricsSink>,
}

impl<E> fmt::Debug for RetryConfig<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetryConfig")
            .field("max_attempts", &self.max_attempts)
            .field("backoff", &self.backoff)
            .field("jitter", &self.jitter)
            .field("total_budget", &self.total_budget)
            .finish_non_exhaustive()
    }
}

impl<E: 'static> RetryConfig<E> {
    /// Create a retry config that retries all errors up to `max_attempts` times.
    ///
    /// `max_attempts` must be at least 1 (the initial attempt counts).
    /// `E` is inferred from the closure passed to [`retry_with`].
    ///
    /// # Errors
    ///
    /// Returns `Err(ConfigError)` if `max_attempts` is 0.
    pub fn new(max_attempts: u32) -> Result<Self, crate::ConfigError> {
        let max_attempts = NonZeroU32::new(max_attempts)
            .ok_or_else(|| crate::ConfigError::new("max_attempts", "must be >= 1"))?;
        Ok(Self {
            max_attempts,
            backoff: BackoffConfig::Fixed(Duration::ZERO),
            jitter: JitterConfig::None,
            total_budget: None,
            classifier: None,
            on_retry: None,
            sink: Arc::new(NoopSink),
        })
    }

    /// Maximum number of attempts, including the initial attempt.
    #[must_use]
    pub const fn max_attempts(&self) -> NonZeroU32 {
        self.max_attempts
    }

    /// Backoff strategy between attempts.
    #[must_use]
    pub const fn backoff_config(&self) -> &BackoffConfig {
        &self.backoff
    }

    /// Optional jitter applied to backoff delays.
    #[must_use]
    pub const fn jitter_config(&self) -> &JitterConfig {
        &self.jitter
    }

    /// Total retry budget, if configured.
    #[must_use]
    pub const fn total_budget_config(&self) -> Option<Duration> {
        self.total_budget
    }

    /// Set the backoff strategy.
    #[must_use]
    pub fn backoff(mut self, backoff: BackoffConfig) -> Self {
        self.backoff = backoff;
        self
    }

    /// Set jitter.
    #[must_use]
    pub const fn jitter(mut self, jitter: JitterConfig) -> Self {
        self.jitter = jitter;
        self
    }

    /// Set a total time budget. The retry loop bounds each operation attempt
    /// and retry sleep by the remaining budget.
    #[must_use]
    pub const fn total_budget(mut self, budget: Duration) -> Self {
        self.total_budget = Some(budget);
        self
    }

    /// Set a custom [`ErrorClassifier`] for retry decisions.
    ///
    /// When set, [`ErrorClassifier::classify`] → [`ErrorClass::is_retryable`]
    /// determines whether to retry, overriding the default
    /// [`Classify::is_retryable()`](nebula_error::Classify::is_retryable).
    #[must_use]
    pub fn with_classifier(mut self, classifier: Arc<dyn ErrorClassifier<E>>) -> Self {
        self.classifier = Some(classifier);
        self
    }

    /// Shorthand: set a bool predicate as the classifier.
    ///
    /// `retry_if(|e| true)` → retry all errors.
    /// `retry_if(|e| false)` → never retry.
    ///
    /// Equivalent to `with_classifier(FnClassifier)` that maps
    /// `true` → [`ErrorClass::Transient`] and `false` → [`ErrorClass::Permanent`].
    #[must_use]
    pub fn retry_if<F>(self, f: F) -> Self
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        self.with_classifier(Arc::new(FnClassifier::new(move |e: &E| {
            if f(e) {
                ErrorClass::Transient
            } else {
                ErrorClass::Permanent
            }
        })))
    }

    /// Register a callback invoked before each retry sleep.
    ///
    /// Receives: `(&error, delay, attempt_number)` where attempt is 1-based.
    #[must_use]
    pub fn on_retry<F>(mut self, f: F) -> Self
    where
        F: Fn(&E, Duration, u32) + Send + Sync + 'static,
    {
        self.on_retry = Some(Arc::new(f));
        self
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Internal constructor that accepts an already validated attempt count.
    pub(crate) fn from_nonzero_attempts(max_attempts: NonZeroU32) -> Self {
        Self {
            max_attempts,
            backoff: BackoffConfig::Fixed(Duration::ZERO),
            jitter: JitterConfig::None,
            total_budget: None,
            classifier: None,
            on_retry: None,
            sink: Arc::new(NoopSink),
        }
    }
}

// ── retry_with ────────────────────────────────────────────────────────────────

/// Execute `f` with retry according to `config`.
///
/// Error classification is automatic via [`Classify`](nebula_error::Classify):
/// - Without a predicate, only errors where
///   [`is_retryable()`](nebula_error::Classify::is_retryable) returns `true` are retried
/// - [`retry_hint().after`](nebula_error::RetryHint::after) is respected as a minimum backoff delay
/// - A [`retry_if`](RetryConfig::retry_if) predicate overrides classification
///
/// # Errors
///
/// Returns `Err(CallError::RetriesExhausted)` when all attempts are exhausted,
/// or `Err(CallError::Operation)` if the error is not retryable.
///
/// # Cancel safety
///
/// Cancel-safe with respect to this crate: dropping the returned future
/// drops the in-flight attempt at its current `.await` and discards all
/// retry bookkeeping (attempt counter, last error, backoff delay) — no
/// crate-owned state is left partially mutated, and no work is detached
/// via `spawn`. Whether a *partially executed* attempt is safe to abandon
/// is the supplied operation's own contract.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::retry::{BackoffConfig, RetryConfig, retry_with};
///
/// # #[derive(Debug)]
/// # struct MyError;
/// # impl std::fmt::Display for MyError {
/// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "err") }
/// # }
/// # impl std::error::Error for MyError {}
/// # impl nebula_error::Classify for MyError {
/// #     fn category(&self) -> nebula_error::ErrorCategory { nebula_error::ErrorCategory::External }
/// #     fn code(&self) -> nebula_error::ErrorCode { nebula_error::ErrorCode::new("DOC:EXAMPLE") }
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let config = RetryConfig::<MyError>::new(3)
///     .expect("max_attempts >= 1")
///     .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));
///
/// let value = retry_with(config, || Box::pin(async { Ok::<_, MyError>(42u32) }))
///     .await
///     .unwrap();
/// assert_eq!(value, 42);
/// # }
/// ```
pub async fn retry_with<T, E, F, Fut>(config: RetryConfig<E>, f: F) -> Result<T, CallError<E>>
where
    E: nebula_error::Classify + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    retry_loop(
        &config,
        f,
        |e: &E| e.is_retryable(),
        |e: &E| e.retry_hint().and_then(|h| h.after),
    )
    .await
}

/// Retry without a [`Classify`](nebula_error::Classify) bound.
///
/// Used by the pipeline and benchmarks. Retries all errors when no predicate
/// is set on the config.
#[doc(hidden)]
pub async fn retry_with_inner<T, E, F, Fut>(config: RetryConfig<E>, f: F) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    retry_loop(&config, f, |_| true, |_| None).await
}

/// Core retry loop shared by [`retry_with`] and [`retry_with_inner`].
///
/// `default_should_retry` is called when no predicate is set on the config.
/// `hint_fn` extracts an optional backoff floor from the error (e.g., `retry_hint().after`).
async fn retry_loop<T, E, F, Fut>(
    config: &RetryConfig<E>,
    mut f: F,
    default_should_retry: impl Fn(&E) -> bool,
    hint_fn: impl Fn(&E) -> Option<Duration>,
) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    let mut last_err: Option<E> = None;
    let mut attempts_executed: u32 = 0;
    let deadline = config.total_budget.map(Deadline::after);
    let max_attempts = config.max_attempts.get();

    for attempt in 0..max_attempts {
        attempts_executed = attempt + 1;
        let attempt_result = if let Some(deadline) = deadline {
            // `Deadline::timeout` re-reads the remaining budget on every
            // attempt, so an instantly-failing operation under a
            // `max_attempts` that far exceeds the budget still stops when the
            // budget does — the budget, not the attempt count, is the bound.
            deadline.timeout(f()).await?
        } else {
            f().await
        };

        match attempt_result {
            Ok(value) => return Ok(value),
            Err(e) => {
                let is_last = attempt + 1 >= max_attempts;

                let should_retry = config.classifier.as_ref().map_or_else(
                    || default_should_retry(&e),
                    |c| c.classify(&e).is_retryable(),
                );

                config.sink.record(ResilienceEvent::RetryAttempt {
                    attempt: attempt + 1,
                    will_retry: !is_last && should_retry,
                });

                if !should_retry {
                    return Err(CallError::Operation(e));
                }

                if is_last {
                    last_err = Some(e);
                    break;
                }

                let mut delay =
                    apply_jitter(config.backoff.delay_for(attempt), &config.jitter, attempt);
                if let Some(floor) = hint_fn(&e) {
                    delay = delay.max(floor);
                }

                if let Some(ref notify) = config.on_retry {
                    notify(&e, delay, attempt + 1);
                }
                last_err = Some(e);

                sleep_with_deadline(delay, deadline).await?;
            },
        }
    }

    last_err.map_or_else(
        || {
            Err(CallError::Timeout(
                deadline.map_or(Duration::ZERO, Deadline::budget),
            ))
        },
        |e| {
            Err(CallError::RetriesExhausted {
                attempts: attempts_executed.max(1),
                last: e,
            })
        },
    )
}

/// Convenience: retry up to `n` times with no backoff.
///
/// Non-retryable errors (authentication, validation, etc.) are skipped
/// automatically via [`Classify::is_retryable()`](nebula_error::Classify::is_retryable).
///
/// # Errors
///
/// Returns `Err(CallError::RetriesExhausted)` when all `n` attempts are exhausted,
/// or `Err(CallError::Operation)` if the error is not retryable.
///
/// # Cancel safety
///
/// Cancel-safe with respect to this crate: dropping the returned future
/// drops the in-flight attempt at its current `.await` and discards all
/// retry bookkeeping (attempt counter, last error, backoff delay) — no
/// crate-owned state is left partially mutated, and no work is detached
/// via `spawn`. Whether a *partially executed* attempt is safe to abandon
/// is the supplied operation's own contract.
///
/// # Examples
///
/// ```rust
/// use std::num::NonZeroU32;
///
/// use nebula_resilience::retry::retry;
///
/// # #[derive(Debug)]
/// # struct MyError;
/// # impl std::fmt::Display for MyError {
/// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "err") }
/// # }
/// # impl std::error::Error for MyError {}
/// # impl nebula_error::Classify for MyError {
/// #     fn category(&self) -> nebula_error::ErrorCategory { nebula_error::ErrorCategory::External }
/// #     fn code(&self) -> nebula_error::ErrorCode { nebula_error::ErrorCode::new("DOC:EXAMPLE") }
/// # }
/// # #[tokio::main]
/// # async fn main() {
/// let attempts = NonZeroU32::new(3).expect("3 != 0");
/// let value = retry(attempts, || Box::pin(async { Ok::<_, MyError>(7u32) }))
///     .await
///     .unwrap();
/// assert_eq!(value, 7);
/// # }
/// ```
pub async fn retry<T, E, F, Fut>(n: NonZeroU32, f: F) -> Result<T, CallError<E>>
where
    E: nebula_error::Classify + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    let config = RetryConfig::<E>::from_nonzero_attempts(n);
    retry_with(config, f).await
}

async fn sleep_with_deadline<E>(
    delay: Duration,
    deadline: Option<Deadline>,
) -> Result<(), CallError<E>> {
    if delay.is_zero() {
        return Ok(());
    }

    let Some(deadline) = deadline else {
        tokio::time::sleep(delay).await;
        return Ok(());
    };

    deadline.sleep(delay).await
}

/// Apply jitter to a base delay.
///
/// When `seed` is set, the jitter is deterministic but varies per `attempt`
/// (seed is mixed with the attempt number to avoid identical jitter across retries).
///
/// Split into leaf dispatcher + outlined `Full` path so that `JitterConfig::None`
/// (the common case) compiles to a 2-instruction function with no register saves.
fn apply_jitter(delay: Duration, jitter: &JitterConfig, attempt: u32) -> Duration {
    match jitter {
        JitterConfig::None => delay,
        JitterConfig::Full { factor, seed } => apply_jitter_full(delay, *factor, *seed, attempt),
    }
}

// Reason: mul_add compiles to `call fma` (~30 cycles) on default target-cpu=x86-64
// which lacks hardware FMA. Explicit multiply+add uses mulsd+addsd (~8 cycles).
#[expect(
    clippy::suboptimal_flops,
    reason = "mul_add emits slow fma call on default x86-64 target; explicit multiply+add is faster"
)]
#[inline(never)]
// Reason: `!(factor > 0.0)` is intentional — it rejects NaN, -0.0, negatives, +0.0,
// and -inf in a single `ucomisd + ja` (2 insns) vs 35-instruction bit decomposition
// that `!is_finite() || <= 0.0` produces. The negated partial-ord is the whole point.
#[expect(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "`!(factor > 0.0)` rejects NaN and negatives in 2 instructions; cleaner than the equivalent is_finite chain"
)]
fn apply_jitter_full(delay: Duration, factor: f64, seed: Option<u64>, attempt: u32) -> Duration {
    if !(factor > 0.0) {
        return delay;
    }

    let base = delay.as_secs_f64();
    let clamped_factor = factor.min(1.0);
    let rand_val = seed.map_or_else(fastrand::f64, |s| {
        fastrand::Rng::with_seed(s.wrapping_add(u64::from(attempt))).f64()
    });
    let total = base + clamped_factor * base * rand_val;
    // total >= 0.0 is guaranteed when base >= 0, factor > 0, rand_val >= 0.
    // Guard against infinity from very large base values.
    if !total.is_finite() {
        return delay;
    }
    Duration::from_secs_f64(total.min(Duration::MAX.as_secs_f64()))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
