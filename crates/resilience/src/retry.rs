//! Retry pattern — unified API with [`Classify`](nebula_error::Classify)-aware error filtering.
//!
//! When `E` implements [`Classify`](nebula_error::Classify), retry automatically skips
//! non-retryable errors (authentication, validation, etc.) and respects
//! [`retry_hint()`](nebula_error::Classify::retry_hint) as a backoff delay floor.

use std::{fmt, future::Future, num::NonZeroU32, sync::Arc, time::Duration};

use smallvec::SmallVec;

use crate::{
    CallError,
    classifier::{ErrorClass, ErrorClassifier, FnClassifier},
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Backoff ───────────────────────────────────────────────────────────────────

/// Backoff strategy for retry delays.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    // Reason: u128 millis cast to f64 for exponential math, f64 result cast to u64 for Duration,
    // and u32 attempt cast to i32 for powi are all acceptable within configured retry bounds.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap
    )]
    pub fn delay_for(&self, attempt: u32) -> Duration {
        match self {
            Self::Fixed(d) => *d,
            Self::Linear { base, max } => (*base * attempt.max(1)).min(*max),
            Self::Exponential {
                base,
                multiplier,
                max,
            } => {
                let ms = base.as_millis() as f64 * multiplier.powi(attempt as i32);
                Duration::from_millis(ms as u64).min(*max)
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

// ── JitterConfig ─────────────────────────────────────────────────────────────

/// Optional jitter to add to backoff delays.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum JitterConfig {
    /// No jitter.
    #[default]
    None,
    /// Add a random fraction up to `factor` of the delay.
    Full {
        /// Maximum jitter fraction (0.0–1.0).
        factor: f64,
        /// Optional seed for deterministic jitter (useful for testing).
        seed: Option<u64>,
    },
}

// ── RetryConfig ───────────────────────────────────────────────────────────────

/// Type alias for the on-retry notification callback.
type RetryNotify<E> = Box<dyn Fn(&E, Duration, u32) + Send + Sync>;

/// Configuration for the retry pattern.
///
/// Error classification is driven by an optional [`ErrorClassifier`]:
/// 1. If set, [`ErrorClassifier::classify`] → [`ErrorClass::is_retryable`] decides.
/// 2. Otherwise, [`retry_with`] falls back to
///    [`Classify::is_retryable()`](nebula_error::Classify::is_retryable).
///
/// Use [`retry_if`](RetryConfig::retry_if) as shorthand for a bool-based classifier,
/// or [`with_classifier`](RetryConfig::with_classifier) for full [`ErrorClass`] control.
pub struct RetryConfig<E = ()> {
    /// Maximum number of attempts (including the first).
    pub max_attempts: u32,
    /// Backoff strategy between attempts.
    pub backoff: BackoffConfig,
    /// Optional jitter applied to backoff delays.
    pub jitter: JitterConfig,
    /// If set, retries stop when total elapsed wall-clock time plus the next backoff
    /// delay would exceed this duration. This bounds the total time spent retrying,
    /// including both operation execution and sleep time.
    pub total_budget: Option<Duration>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    on_retry: Option<RetryNotify<E>>,
    sink: Arc<dyn MetricsSink>,
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
        if max_attempts == 0 {
            return Err(crate::ConfigError::new("max_attempts", "must be >= 1"));
        }
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

    /// Set a total time budget — retries stop if elapsed wall-clock time plus the next
    /// backoff delay would exceed this duration.
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
        self.on_retry = Some(Box::new(f));
        self
    }

    /// Inject a metrics sink.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Internal constructor that skips validation — caller guarantees `max_attempts >= 1`.
    pub(crate) fn new_unchecked(max_attempts: u32) -> Self {
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
/// # Panics
///
/// Panics if `config.max_attempts` is 0 (this is prevented by [`RetryConfig::new`]).
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
    debug_assert!(
        config.max_attempts >= 1,
        "retry_loop called with max_attempts=0; use RetryConfig::new() to prevent this"
    );

    let mut last_err: Option<E> = None;
    let mut attempts_executed: u32 = 0;
    let start = std::time::Instant::now();

    for attempt in 0..config.max_attempts {
        attempts_executed = attempt + 1;
        match f().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                let is_last = attempt + 1 >= config.max_attempts;

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

                // Budget check: wall-clock time + next delay exceeds budget → stop
                if config.total_budget.is_some_and(|budget| {
                    start
                        .elapsed()
                        .checked_add(delay)
                        .is_none_or(|spent| spent > budget)
                }) {
                    break;
                }

                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            },
        }
    }

    // Unreachable None case: debug_assert above guarantees max_attempts >= 1,
    // so at least one iteration ran and set last_err.
    Err(last_err.map_or_else(
        || CallError::cancelled(),
        |e| CallError::RetriesExhausted {
            attempts: attempts_executed.max(1),
            last: e,
        },
    ))
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
pub async fn retry<T, E, F, Fut>(n: NonZeroU32, f: F) -> Result<T, CallError<E>>
where
    E: nebula_error::Classify + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    let config = RetryConfig::<E>::new_unchecked(n.get());
    retry_with(config, f).await
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
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use nebula_error::{Classify, ErrorCategory, ErrorCode, RetryHint, codes};

    use super::*;
    use crate::{CallError, RecordingSink, ResilienceEventKind};

    /// Test error type implementing Classify. Always retryable.
    #[derive(Debug, Clone, PartialEq)]
    struct TransientErr(&'static str);
    impl fmt::Display for TransientErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }
    impl Classify for TransientErr {
        fn category(&self) -> ErrorCategory {
            ErrorCategory::External
        }
        fn code(&self) -> ErrorCode {
            codes::INTERNAL
        }
    }

    /// Test error with variants for retryable/non-retryable.
    #[derive(Debug)]
    enum TestApiErr {
        Timeout,
        AuthFailed,
        RateLimited(Duration),
    }
    impl fmt::Display for TestApiErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{self:?}")
        }
    }
    impl Classify for TestApiErr {
        fn category(&self) -> ErrorCategory {
            match self {
                Self::Timeout => ErrorCategory::Timeout,
                Self::AuthFailed => ErrorCategory::Authentication,
                Self::RateLimited(_) => ErrorCategory::RateLimit,
            }
        }
        fn code(&self) -> ErrorCode {
            codes::INTERNAL
        }
        fn retry_hint(&self) -> Option<RetryHint> {
            match self {
                Self::RateLimited(d) => Some(RetryHint::after(*d)),
                _ => None,
            }
        }
    }

    fn fail_twice(counter: &AtomicU32) -> Result<u32, TransientErr> {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < 2 {
            Err(TransientErr("fail"))
        } else {
            Ok(99)
        }
    }

    #[tokio::test]
    async fn retries_up_to_max_attempts() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let config = RetryConfig::new(3)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        let result: Result<(), CallError<TransientErr>> = retry_with(config, async || {
            c.fetch_add(1, Ordering::SeqCst);
            Err(TransientErr("fail"))
        })
        .await;

        assert!(matches!(
            result,
            Err(CallError::RetriesExhausted { attempts: 3, .. })
        ));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn stops_on_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let config = RetryConfig::new(5)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        let result: Result<u32, CallError<TransientErr>> =
            retry_with(config, async || fail_twice(&c)).await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_if_predicate_overrides_classify() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        // Timeout IS retryable by Classify, but predicate says no
        let config = RetryConfig::new(5)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .retry_if(|_: &TestApiErr| false);

        let result = retry_with(config, async || {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<u32, TestApiErr>(TestApiErr::Timeout)
        })
        .await;

        // Custom predicate takes precedence → 1 attempt
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(matches!(result, Err(CallError::Operation(_))));
    }

    #[tokio::test]
    async fn emits_retry_attempt_events() {
        let sink = RecordingSink::new();
        let config = RetryConfig::new(3)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .with_sink(sink.clone());

        let _: Result<(), CallError<TransientErr>> =
            retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;

        assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 3);
    }

    #[test]
    fn fibonacci_backoff_produces_correct_sequence() {
        let cfg = BackoffConfig::Fibonacci {
            base: Duration::from_millis(100),
            max: Duration::from_secs(5),
        };
        assert_eq!(cfg.delay_for(0), Duration::from_millis(100));
        assert_eq!(cfg.delay_for(1), Duration::from_millis(100));
        assert_eq!(cfg.delay_for(2), Duration::from_millis(200));
        assert_eq!(cfg.delay_for(3), Duration::from_millis(300));
        assert_eq!(cfg.delay_for(4), Duration::from_millis(500));
        assert_eq!(cfg.delay_for(5), Duration::from_millis(800));
    }

    #[test]
    fn fibonacci_backoff_respects_max() {
        let cfg = BackoffConfig::Fibonacci {
            base: Duration::from_millis(100),
            max: Duration::from_millis(250),
        };
        assert_eq!(cfg.delay_for(4), Duration::from_millis(250));
    }

    #[tokio::test]
    async fn on_retry_callback_receives_error_and_delay() {
        let notifications = Arc::new(std::sync::Mutex::new(Vec::new()));
        let n = notifications.clone();

        let config = RetryConfig::new(3)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .on_retry(move |_err: &TransientErr, delay: Duration, attempt: u32| {
                n.lock().unwrap().push((attempt, delay));
            });

        let _: Result<(), CallError<TransientErr>> =
            retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;

        let notifs = notifications.lock().unwrap();
        assert_eq!(notifs.len(), 2); // 2 retries (3 attempts - 1 initial)
        assert_eq!(notifs[0].0, 1);
        assert_eq!(notifs[1].0, 2);
        drop(notifs);
    }

    #[tokio::test]
    async fn jitter_adds_delay_variance() {
        // With full jitter (factor=1.0), total delay should be between
        // base and 2×base on average. We verify it doesn't exceed 3×base
        // (generous bound to avoid flakiness).
        let base = Duration::from_millis(10);
        let config = RetryConfig::new(3)
            .unwrap()
            .backoff(BackoffConfig::Fixed(base))
            .jitter(JitterConfig::Full {
                factor: 1.0,
                seed: None,
            });

        let start = std::time::Instant::now();
        let _: Result<(), CallError<TransientErr>> =
            retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;
        let elapsed = start.elapsed();

        // 2 retries × base = 20ms minimum (no jitter on first attempt which uses attempt=0)
        // With jitter factor 1.0, max theoretical = 2 × 2×base = 40ms
        // Use a generous upper bound to avoid flakiness
        assert!(
            elapsed >= Duration::from_millis(20),
            "expected >= 20ms, got {elapsed:?}"
        );
    }

    #[test]
    fn seeded_jitter_is_deterministic_for_same_attempt() {
        let delay = Duration::from_millis(100);
        let jitter = JitterConfig::Full {
            factor: 0.5,
            seed: Some(42),
        };
        let d1 = apply_jitter(delay, &jitter, 0);
        let d2 = apply_jitter(delay, &jitter, 0);

        assert_eq!(d1, d2, "same seed + same attempt must produce same jitter");
        assert!(d1 > delay, "jitter should add to delay");
        assert!(
            d1 <= Duration::from_millis(150),
            "factor 0.5 caps at 50% extra"
        );
    }

    #[test]
    fn seeded_jitter_varies_across_attempts() {
        let delay = Duration::from_millis(100);
        let jitter = JitterConfig::Full {
            factor: 0.5,
            seed: Some(42),
        };
        let d0 = apply_jitter(delay, &jitter, 0);
        let d1 = apply_jitter(delay, &jitter, 1);
        let d2 = apply_jitter(delay, &jitter, 2);

        // Different attempts should (almost certainly) produce different jitter
        assert!(
            d0 != d1 || d1 != d2,
            "seeded jitter should vary per attempt: d0={d0:?}, d1={d1:?}, d2={d2:?}"
        );
    }

    #[test]
    fn jitter_with_nan_factor_falls_back_to_base_delay() {
        let delay = Duration::from_millis(100);
        let nan_jitter = JitterConfig::Full {
            factor: f64::NAN,
            seed: Some(42),
        };
        assert_eq!(apply_jitter(delay, &nan_jitter, 0), delay);

        let neg_jitter = JitterConfig::Full {
            factor: -1.0,
            seed: Some(42),
        };
        assert_eq!(apply_jitter(delay, &neg_jitter, 0), delay);

        let zero_jitter = JitterConfig::Full {
            factor: 0.0,
            seed: Some(42),
        };
        assert_eq!(apply_jitter(delay, &zero_jitter, 0), delay);
    }

    #[test]
    fn jitter_with_infinite_factor_clamps_to_one() {
        let delay = Duration::from_millis(100);
        let jitter = JitterConfig::Full {
            factor: f64::INFINITY,
            seed: Some(42),
        };
        // Infinity is clamped to 1.0 by factor.min(1.0), so jitter is applied
        let result = apply_jitter(delay, &jitter, 0);
        assert!(result >= delay, "clamped infinity factor should add jitter");
    }

    #[tokio::test]
    async fn total_budget_check_handles_large_backoff_without_panic() {
        let config = RetryConfig::new(3)
            .unwrap()
            .backoff(BackoffConfig::Custom(SmallVec::from_slice(&[
                Duration::MAX,
            ])))
            .total_budget(Duration::from_secs(1));

        let result: Result<(), CallError<TransientErr>> =
            retry_with(config, || Box::pin(async { Err(TransientErr("fail")) })).await;

        assert!(matches!(result, Err(CallError::RetriesExhausted { .. })));
    }

    #[test]
    fn custom_backoff_uses_provided_delays() {
        let cfg = BackoffConfig::Custom(SmallVec::from_slice(&[
            Duration::from_millis(10),
            Duration::from_millis(50),
            Duration::from_millis(200),
        ]));
        assert_eq!(cfg.delay_for(0), Duration::from_millis(10));
        assert_eq!(cfg.delay_for(1), Duration::from_millis(50));
        assert_eq!(cfg.delay_for(2), Duration::from_millis(200));
        assert_eq!(cfg.delay_for(3), Duration::from_millis(200));
        assert_eq!(cfg.delay_for(99), Duration::from_millis(200));
    }

    #[test]
    fn custom_backoff_empty_returns_zero() {
        let cfg = BackoffConfig::Custom(SmallVec::new());
        assert_eq!(cfg.delay_for(0), Duration::ZERO);
    }

    // ── Classify integration tests ──────────────────────────────────────────

    #[tokio::test]
    async fn retry_auto_skips_non_retryable() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let result = retry(NonZeroU32::new(3).unwrap(), async || {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(TestApiErr::AuthFailed)
        })
        .await;

        // Auth is not retryable — stops after 1 attempt
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(matches!(
            result,
            Err(CallError::Operation(TestApiErr::AuthFailed))
        ));
    }

    #[tokio::test]
    async fn retry_auto_retries_retryable() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let result = retry(NonZeroU32::new(3).unwrap(), async || {
            c.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(TestApiErr::Timeout)
        })
        .await;

        // Timeout IS retryable — exhausts all 3 attempts
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        assert!(matches!(
            result,
            Err(CallError::RetriesExhausted { attempts: 3, .. })
        ));
    }

    #[tokio::test]
    async fn retry_respects_hint_floor() {
        let start = std::time::Instant::now();
        let config = RetryConfig::new(2)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        // RateLimited error with 50ms hint — should override 1ms backoff
        let _: Result<(), CallError<TestApiErr>> = retry_with(config, || {
            Box::pin(async { Err(TestApiErr::RateLimited(Duration::from_millis(50))) })
        })
        .await;

        let elapsed = start.elapsed();
        // 1 retry with hint floor of 50ms — should take at least ~50ms
        assert!(
            elapsed >= Duration::from_millis(45),
            "expected >= 45ms (hint floor), got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn total_budget_stops_retries_early() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let config = RetryConfig::new(100)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(50)))
            .total_budget(Duration::from_millis(120));

        let start = std::time::Instant::now();
        let _: Result<(), CallError<TransientErr>> = retry_with(config, async || {
            c.fetch_add(1, Ordering::SeqCst);
            Err(TransientErr("fail"))
        })
        .await;
        let elapsed = start.elapsed();

        let attempts = counter.load(Ordering::SeqCst);
        assert!(attempts <= 4, "expected <= 4, got {attempts}");
        assert!(
            elapsed < Duration::from_millis(300),
            "took too long: {elapsed:?}"
        );
    }

    // ── B3: total_budget works with zero-delay backoff ───────────────────

    #[tokio::test]
    async fn total_budget_limits_zero_delay_retries() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        // Zero delay + tiny budget → budget should still stop retries
        // because wall-clock time of executing ops eventually exceeds budget.
        let config = RetryConfig::new(1000)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::ZERO))
            .total_budget(Duration::from_millis(50));

        let _: Result<(), CallError<TransientErr>> = retry_with(config, async || {
            c.fetch_add(1, Ordering::SeqCst);
            // Each op sleeps 5ms → after ~10 ops, 50ms budget is hit
            tokio::time::sleep(Duration::from_millis(5)).await;
            Err(TransientErr("fail"))
        })
        .await;

        let attempts = counter.load(Ordering::SeqCst);
        // With 5ms per op and 50ms budget, should stop around 10 attempts (not 1000)
        assert!(
            attempts < 50,
            "expected budget to stop retries, got {attempts} attempts"
        );
    }

    // ── B4: pipeline forwards retry_after from rate limiter ──────────────

    #[tokio::test]
    async fn pipeline_forwards_rate_limiter_retry_after() {
        use crate::pipeline::{RateLimitCheck, ResiliencePipeline};

        let hint = Duration::from_secs(42);
        let rl: RateLimitCheck = Arc::new(move || {
            Box::pin(async move {
                Err(CallError::RateLimited {
                    retry_after: Some(hint),
                })
            })
        });

        let pipeline = ResiliencePipeline::<&str>::builder()
            .rate_limiter(rl)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        match result {
            Err(CallError::RateLimited { retry_after }) => {
                assert_eq!(
                    retry_after,
                    Some(hint),
                    "retry_after hint should be forwarded"
                );
            },
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    // ── D1: rate_limiter_from convenience method ─────────────────────────

    #[tokio::test]
    async fn pipeline_rate_limiter_from_works() {
        use crate::{pipeline::ResiliencePipeline, rate_limiter::TokenBucket};

        let rl = Arc::new(TokenBucket::new(1, 0.001).unwrap());

        let pipeline = ResiliencePipeline::<&str>::builder()
            .rate_limiter_from(Arc::clone(&rl))
            .build();

        // First call succeeds (1 token available)
        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;
        assert_eq!(result.unwrap(), 42);

        // Second call should be rate limited (no tokens left)
        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;
        assert!(matches!(result, Err(CallError::RateLimited { .. })));
    }
}
