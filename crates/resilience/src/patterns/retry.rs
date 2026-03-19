//! Retry pattern — unified API, predicate-based error classification.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    CallError,
    clock::{Clock, SystemClock},
    observability::sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Backoff ───────────────────────────────────────────────────────────────────

/// Backoff strategy for retry delays.
#[derive(Debug, Clone)]
pub enum BackoffConfig {
    /// Same delay between every attempt.
    Fixed(Duration),
    /// Linearly increasing delay, capped at `max`.
    Linear { base: Duration, max: Duration },
    /// Exponentially increasing delay, capped at `max`.
    Exponential {
        base: Duration,
        multiplier: f64,
        max: Duration,
    },
}

impl BackoffConfig {
    /// Standard exponential backoff: 100ms base, 2× multiplier, 30s cap.
    pub fn exponential_default() -> Self {
        Self::Exponential {
            base: Duration::from_millis(100),
            multiplier: 2.0,
            max: Duration::from_secs(30),
        }
    }

    pub(crate) fn delay_for(&self, attempt: u32) -> Duration {
        match self {
            Self::Fixed(d) => *d,
            Self::Linear { base, max } => (*base * attempt).min(*max),
            Self::Exponential {
                base,
                multiplier,
                max,
            } => {
                let ms = base.as_millis() as f64 * multiplier.powi(attempt as i32);
                Duration::from_millis(ms as u64).min(*max)
            }
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
    Full { factor: f64 },
}

// ── RetryConfig ───────────────────────────────────────────────────────────────

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
    predicate: Option<Box<dyn Fn(&E) -> bool + Send + Sync>>,
    sink: Arc<dyn MetricsSink>,
    clock: Arc<dyn Clock>,
}

impl<E: 'static> RetryConfig<E> {
    /// Create a retry config that retries all errors up to `max_attempts` times.
    ///
    /// `E` is inferred from the closure passed to [`retry_with`].
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            backoff: BackoffConfig::Fixed(Duration::ZERO),
            jitter: JitterConfig::None,
            predicate: None,
            sink: Arc::new(NoopSink),
            clock: Arc::new(SystemClock),
        }
    }
    /// Set the backoff strategy.
    pub fn backoff(mut self, backoff: BackoffConfig) -> Self {
        self.backoff = backoff;
        self
    }

    /// Set jitter.
    pub fn jitter(mut self, jitter: JitterConfig) -> Self {
        self.jitter = jitter;
        self
    }

    /// Only retry when this predicate returns `true`.
    ///
    /// Without a predicate, all errors are retried.
    pub fn retry_if<F>(mut self, f: F) -> Self
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        self.predicate = Some(Box::new(f));
        self
    }

    /// Inject a metrics sink.
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    fn should_retry(&self, err: &E) -> bool {
        self.predicate.as_ref().map_or(true, |p| p(err))
    }
}

// ── retry_with ────────────────────────────────────────────────────────────────

/// Execute `f` with retry according to `config`.
///
/// - If all attempts fail and retrying was allowed: returns `Err(CallError::RetriesExhausted)`
/// - If the predicate says don't retry: returns `Err(CallError::Operation)` immediately
pub async fn retry_with<T, E, F>(config: RetryConfig<E>, mut f: F) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    let mut last_err: Option<E> = None;

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

                // Will retry — apply backoff and continue
                last_err = Some(e);
                let delay = config.backoff.delay_for(attempt);
                if !delay.is_zero() {
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
pub async fn retry<T, E, F>(n: u32, f: F) -> Result<T, CallError<E>>
where
    E: 'static,
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    retry_with(RetryConfig::<E>::new(n), f).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, RecordingSink};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn retries_up_to_max_attempts() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let config = RetryConfig::new(3).backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

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
        let config = RetryConfig::new(5).backoff(BackoffConfig::Fixed(Duration::from_millis(1)));

        let result: Result<u32, CallError<&str>> = retry_with(config, || {
            let c = c.clone();
            Box::pin(async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 { Err("fail") } else { Ok(99u32) }
            })
        })
        .await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_if_predicate_stops_on_permanent_error() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        #[derive(Debug)]
        enum MyErr {
            Transient,
            Permanent,
        }

        let config = RetryConfig::new(5)
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
            .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
            .with_sink(sink.clone());

        let _: Result<(), CallError<&str>> =
            retry_with(config, || Box::pin(async { Err("fail") })).await;

        assert_eq!(sink.count("retry_attempt"), 3);
    }
}
