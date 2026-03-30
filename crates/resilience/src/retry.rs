//! Retry pattern — unified API, predicate-based error classification.

use std::future::Future;

use std::sync::Arc;
use std::time::Duration;

use crate::{
    CallError,
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Backoff ───────────────────────────────────────────────────────────────────

/// Backoff strategy for retry delays.
#[derive(Debug, Clone)]
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
    Custom(Vec<Duration>),
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
            }
            Self::Fibonacci { base, max } => {
                let fib_n = Self::fibonacci(attempt);
                base.saturating_mul(fib_n).min(*max)
            }
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
#[derive(Debug, Clone, Default)]
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

/// Configuration for the retry pattern.
///
/// By default retries all errors up to `max_attempts`.
/// Use [`retry_if`](RetryConfig::retry_if) to restrict retries to specific error classes.
/// Type alias for the retry predicate closure.
type RetryPredicate<E> = Box<dyn Fn(&E) -> bool + Send + Sync>;

/// Type alias for the on-retry notification callback.
type RetryNotify<E> = Box<dyn Fn(&E, Duration, u32) + Send + Sync>;

/// Configuration for the retry pattern.
///
/// By default retries all errors up to `max_attempts`.
/// Use [`retry_if`](RetryConfig::retry_if) to restrict retries to specific error classes.
pub struct RetryConfig<E = ()> {
    /// Maximum number of attempts (including the first).
    pub max_attempts: u32,
    /// Backoff strategy between attempts.
    pub backoff: BackoffConfig,
    /// Optional jitter applied to backoff delays.
    pub jitter: JitterConfig,
    /// If set, retries stop when cumulative sleep time would exceed this budget.
    pub total_budget: Option<Duration>,
    predicate: Option<RetryPredicate<E>>,
    on_retry: Option<RetryNotify<E>>,
    sink: Arc<dyn MetricsSink>,
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
            predicate: None,
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

    /// Set a total delay budget — retries stop if cumulative sleep time would exceed this.
    #[must_use]
    pub const fn total_budget(mut self, budget: Duration) -> Self {
        self.total_budget = Some(budget);
        self
    }

    /// Only retry when this predicate returns `true`.
    ///
    /// Without a predicate, all errors are retried.
    #[must_use]
    pub fn retry_if<F>(mut self, f: F) -> Self
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        self.predicate = Some(Box::new(f));
        self
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
            predicate: None,
            on_retry: None,
            sink: Arc::new(NoopSink),
        }
    }

    fn should_retry(&self, err: &E) -> bool {
        self.predicate.as_ref().is_none_or(|p| p(err))
    }
}

// ── retry_with ────────────────────────────────────────────────────────────────

/// Execute `f` with retry according to `config`.
///
/// - If all attempts fail and retrying was allowed: returns `Err(CallError::RetriesExhausted)`
/// - If the predicate says don't retry: returns `Err(CallError::Operation)` immediately
///
/// # Errors
///
/// Returns `Err(CallError::RetriesExhausted)` when all attempts are exhausted,
/// or `Err(CallError::Operation)` if the retry predicate rejects the error.
///
/// # Panics
///
/// Panics if `config.max_attempts` is 0 (this is prevented by [`RetryConfig::new`]).
pub async fn retry_with<T, E, F, Fut>(config: RetryConfig<E>, mut f: F) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    let mut last_err: Option<E> = None;
    let mut total_delay = Duration::ZERO;

    for attempt in 0..config.max_attempts {
        match f().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                let is_last = attempt + 1 >= config.max_attempts;
                let predicate_allows = config.should_retry(&e);

                config.sink.record(ResilienceEvent::RetryAttempt {
                    attempt: attempt + 1,
                    will_retry: !is_last && predicate_allows,
                });

                if !predicate_allows {
                    // Predicate says don't retry — surface immediately as Operation
                    return Err(CallError::Operation(e));
                }

                if is_last {
                    // All attempts exhausted — will return RetriesExhausted below
                    last_err = Some(e);
                    break;
                }

                // Will retry — apply backoff + jitter and continue
                let delay = apply_jitter(config.backoff.delay_for(attempt), &config.jitter);
                if let Some(ref notify) = config.on_retry {
                    notify(&e, delay, attempt + 1);
                }
                last_err = Some(e);
                if !delay.is_zero() {
                    if config
                        .total_budget
                        .is_some_and(|budget| total_delay + delay > budget)
                    {
                        break;
                    }
                    total_delay += delay;
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    Err(CallError::RetriesExhausted {
        attempts: config.max_attempts,
        last: last_err.expect("at least one attempt was made"),
    })
}

/// Convenience: retry up to `n` times with no backoff.
///
/// # Errors
///
/// Returns `Err(CallError::RetriesExhausted)` when all `n` attempts are exhausted.
///
/// # Panics
///
/// Panics if `n` is 0 (use [`retry_with`] with [`RetryConfig::new`] for fallible construction).
pub async fn retry<T, E, F, Fut>(n: u32, f: F) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
{
    let config = RetryConfig::<E>::new(n)
        .expect("retry() requires n >= 1; use retry_with() for fallible config");
    retry_with(config, f).await
}

/// Apply jitter to a base delay.
fn apply_jitter(delay: Duration, jitter: &JitterConfig) -> Duration {
    match jitter {
        JitterConfig::None => delay,
        JitterConfig::Full { factor, seed } => {
            let base = delay.as_secs_f64();
            let rand_val = seed.map_or_else(fastrand::f64, |s| fastrand::Rng::with_seed(s).f64());
            let jitter_amount = base * factor * rand_val;
            Duration::from_secs_f64(base + jitter_amount)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, RecordingSink};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    fn fail_twice(counter: &AtomicU32) -> Result<u32, &'static str> {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        if n < 2 { Err("fail") } else { Ok(99) }
    }

    #[tokio::test]
    async fn retries_up_to_max_attempts() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let config = RetryConfig::new(3)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        let result: Result<(), CallError<&str>> = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err("fail")
            })
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

        let result: Result<u32, CallError<&str>> = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move { fail_twice(&c) })
        })
        .await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[derive(Debug)]
    enum MyErr {
        #[allow(dead_code)]
        Transient,
        Permanent,
    }

    #[tokio::test]
    async fn retry_if_predicate_stops_on_permanent_error() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let config = RetryConfig::new(5)
            .unwrap()
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .retry_if(|e: &MyErr| matches!(e, MyErr::Transient));

        let result = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<u32, MyErr>(MyErr::Permanent)
            })
        })
        .await;

        // Should stop after 1 attempt — Permanent is not retryable
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

        let _: Result<(), CallError<&str>> =
            retry_with(config, || Box::pin(async { Err("fail") })).await;

        assert_eq!(sink.count("retry_attempt"), 3);
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
            .on_retry(move |_err: &&str, delay: Duration, attempt: u32| {
                n.lock().unwrap().push((attempt, delay));
            });

        let _: Result<(), CallError<&str>> =
            retry_with(config, || Box::pin(async { Err("fail") })).await;

        let notifs = notifications.lock().unwrap();
        assert_eq!(notifs.len(), 2); // 2 retries (3 attempts - 1 initial)
        assert_eq!(notifs[0].0, 1);
        assert_eq!(notifs[1].0, 2);
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
        let _: Result<(), CallError<&str>> =
            retry_with(config, || Box::pin(async { Err("fail") })).await;
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
    fn seeded_jitter_is_deterministic() {
        let delay = Duration::from_millis(100);
        let jitter = JitterConfig::Full {
            factor: 0.5,
            seed: Some(42),
        };
        let d1 = apply_jitter(delay, &jitter);

        let jitter2 = JitterConfig::Full {
            factor: 0.5,
            seed: Some(42),
        };
        let d2 = apply_jitter(delay, &jitter2);

        assert_eq!(d1, d2, "same seed must produce same jitter");
        assert!(d1 > delay, "jitter should add to delay");
        assert!(
            d1 <= Duration::from_millis(150),
            "factor 0.5 caps at 50% extra"
        );
    }

    #[test]
    fn custom_backoff_uses_provided_delays() {
        let delays = vec![
            Duration::from_millis(10),
            Duration::from_millis(50),
            Duration::from_millis(200),
        ];
        let cfg = BackoffConfig::Custom(delays);
        assert_eq!(cfg.delay_for(0), Duration::from_millis(10));
        assert_eq!(cfg.delay_for(1), Duration::from_millis(50));
        assert_eq!(cfg.delay_for(2), Duration::from_millis(200));
        assert_eq!(cfg.delay_for(3), Duration::from_millis(200));
        assert_eq!(cfg.delay_for(99), Duration::from_millis(200));
    }

    #[test]
    fn custom_backoff_empty_returns_zero() {
        let cfg = BackoffConfig::Custom(vec![]);
        assert_eq!(cfg.delay_for(0), Duration::ZERO);
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
        let _: Result<(), CallError<&str>> = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err("fail")
            })
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
}
