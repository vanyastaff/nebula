//! `ResiliencePipeline` — compose multiple resilience patterns into a single call chain.
//!
//! Recommended layer order (outermost → innermost):
//! `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`
//!
//! Layers are applied in the order added: first added = outermost.

use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use parking_lot::Mutex;

use crate::{
    CallError,
    bulkhead::Bulkhead,
    circuit_breaker::{CircuitBreaker, Outcome, ProbeGuard},
    classifier::{ErrorClass, ErrorClassifier, FnClassifier},
    retry::{RetryConfig, retry_with_inner},
    sink::{MetricsSink, NoopSink, ResilienceEvent},
};

// ── Execution ────────────────────────────────────────────────────────────────
//
// Steps are processed recursively by `run_operation_with_shells`. Each step
// type is handled exactly once, in order:
//
// - LoadShed / RateLimiter: checked before recursing to inner steps.
// - CircuitBreaker: `try_acquire()` + `ProbeGuard` + `record_outcome()`.
// - Bulkhead: `acquire()` permit held for the inner scope.
// - Timeout / Retry: wrap the remainder of the pipeline.
//
// `run_operation_with_shells` wraps every recursive call in `Box::pin`
// (required because the async fn is recursive). Timeout and Retry add
// additional overhead: Timeout for `tokio::time::timeout` wrapping,
// Retry for the back-off loop with `retry_with_inner`.

/// Async predicate for rate limiting — returns `Ok(())` or `Err(CallError::RateLimited)`.
pub type RateLimitCheck =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), CallError<()>>> + Send>> + Send + Sync>;

/// Predicate for load shedding — returns `true` to shed the request.
pub type LoadShedPredicate = Arc<dyn Fn() -> bool + Send + Sync>;

// ── Steps ─────────────────────────────────────────────────────────────────────

enum Step<E: 'static> {
    Timeout(Duration),
    Retry(Box<RetryConfig<E>>),
    CircuitBreaker(Arc<CircuitBreaker>),
    Bulkhead(Arc<Bulkhead>),
    RateLimiter(RateLimitCheck),
    LoadShed(LoadShedPredicate),
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Builder for [`ResiliencePipeline`].
pub struct PipelineBuilder<E: 'static> {
    steps: Vec<Step<E>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Option<Arc<dyn MetricsSink>>,
}

impl<E: 'static> fmt::Debug for PipelineBuilder<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PipelineBuilder")
            .field("steps", &self.steps.len())
            .finish_non_exhaustive()
    }
}

impl<E: Send + 'static> Default for PipelineBuilder<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Send + 'static> PipelineBuilder<E> {
    /// Create a new builder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            steps: Vec::new(),
            classifier: None,
            sink: None,
        }
    }

    /// Set an [`ErrorClassifier`] for the pipeline.
    ///
    /// When set, the circuit breaker step uses
    /// [`call_with_classifier`](CircuitBreaker::call_with_classifier) instead of
    /// [`call`](CircuitBreaker::call). The retry step combines this with any classifier on
    /// [`RetryConfig`](crate::retry::RetryConfig) (per-retry classifier wins for retry
    /// decisions; the pipeline classifier still applies to the circuit breaker).
    ///
    /// Without a **pipeline** classifier, operation errors that reach the circuit breaker are
    /// all counted as [`Failure`](crate::circuit_breaker::Outcome::Failure) for CB state. The
    /// retry step, when neither a pipeline nor a per-retry classifier is set, treats every
    /// operation error as retryable (the historical pipeline default). For
    /// [`Classify`](nebula_error::Classify) semantics like standalone
    /// [`retry_with`](crate::retry::retry_with), set [`classify_errors()`](Self::classify_errors)
    /// and/or per-retry [`retry_if`](crate::retry::RetryConfig::retry_if) /
    /// [`with_classifier`](crate::retry::RetryConfig::with_classifier).
    #[must_use]
    pub fn classifier(mut self, classifier: Arc<dyn ErrorClassifier<E>>) -> Self {
        self.classifier = Some(classifier);
        self
    }

    /// Inject a metrics sink for pipeline-level timeout / rate-limit / load-shed events.
    #[must_use]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Some(Arc::new(sink));
        self
    }

    /// Add a timeout step (outermost wrapper if added first).
    #[must_use]
    pub fn timeout(mut self, d: Duration) -> Self {
        self.steps.push(Step::Timeout(d));
        self
    }

    /// Add a retry step.
    ///
    /// **Note:** Pipeline retry uses `BackoffConfig` for delay timing.
    /// [`retry_hint().after`](nebula_error::Classify::retry_hint) from individual errors
    /// is **not** applied in the pipeline path — use [`retry_with`](crate::retry::retry_with)
    /// directly if retry-after hints are needed.
    #[must_use]
    pub fn retry(mut self, config: RetryConfig<E>) -> Self {
        self.steps.push(Step::Retry(Box::new(config)));
        self
    }

    /// Add a circuit breaker step.
    #[must_use]
    pub fn circuit_breaker(mut self, cb: Arc<CircuitBreaker>) -> Self {
        self.steps.push(Step::CircuitBreaker(cb));
        self
    }

    /// Add a bulkhead step.
    #[must_use]
    pub fn bulkhead(mut self, bh: Arc<Bulkhead>) -> Self {
        self.steps.push(Step::Bulkhead(bh));
        self
    }

    /// Add a rate limiter step using a concrete `RateLimiter` implementation.
    ///
    /// This is the ergonomic way to add rate limiting — it handles the closure
    /// bridging automatically:
    ///
    /// ```rust,ignore
    /// let rl = Arc::new(TokenBucket::new(100, 10.0).unwrap());
    /// builder.rate_limiter_from(rl)
    /// ```
    #[must_use]
    pub fn rate_limiter_from<RL: crate::RateLimiter + 'static>(self, rl: Arc<RL>) -> Self {
        let check: RateLimitCheck = Arc::new(move || {
            let rl = Arc::clone(&rl);
            Box::pin(async move { rl.acquire().await })
        });
        self.rate_limiter(check)
    }

    /// Add a rate limiter step with a custom check closure.
    ///
    /// Prefer [`rate_limiter_from`](Self::rate_limiter_from) for standard `RateLimiter`
    /// implementations. Use this for custom bridging logic.
    #[must_use]
    pub fn rate_limiter(mut self, check: RateLimitCheck) -> Self {
        self.steps.push(Step::RateLimiter(check));
        self
    }

    /// Add a load shedding step. The predicate returns `true` to shed the request.
    #[must_use]
    pub fn load_shed(mut self, predicate: LoadShedPredicate) -> Self {
        self.steps.push(Step::LoadShed(predicate));
        self
    }

    /// Build the pipeline, emitting a tracing warning if layer order is suboptimal.
    #[must_use]
    pub fn build(self) -> ResiliencePipeline<E> {
        validate_order(&self.steps);
        ResiliencePipeline {
            steps: Arc::new(self.steps),
            classifier: self.classifier,
            sink: self.sink.unwrap_or_else(|| Arc::new(NoopSink)),
        }
    }
}

/// Convenience: set [`NebulaClassifier`](crate::classifier::NebulaClassifier)
/// when `E` implements [`Classify`](nebula_error::Classify).
impl<E: nebula_error::Classify + Send + Sync + 'static> PipelineBuilder<E> {
    /// Use [`NebulaClassifier`](crate::classifier::NebulaClassifier) to automatically
    /// map [`ErrorCategory`](nebula_error::ErrorCategory) to
    /// [`ErrorClass`](crate::classifier::ErrorClass).
    ///
    /// This is the recommended default for pipelines where `E: Classify`.
    #[must_use]
    pub fn classify_errors(self) -> Self {
        self.classifier(Arc::new(crate::classifier::NebulaClassifier))
    }
}

fn validate_order<E>(steps: &[Step<E>]) {
    let names: Vec<&str> = steps
        .iter()
        .map(|s| match s {
            Step::Timeout(_) => "timeout",
            Step::Retry(_) => "retry",
            Step::CircuitBreaker(_) => "circuit_breaker",
            Step::Bulkhead(_) => "bulkhead",
            Step::RateLimiter(_) => "rate_limiter",
            Step::LoadShed(_) => "load_shed",
        })
        .collect();

    let retry_pos = names.iter().position(|&n| n == "retry");
    let timeout_pos = names.iter().position(|&n| n == "timeout");
    let rate_limiter_pos = names.iter().position(|&n| n == "rate_limiter");

    if let (Some(r), Some(t)) = (retry_pos, timeout_pos)
        && t > r
    {
        tracing::warn!(
            "ResiliencePipeline: timeout is inside retry (each attempt gets its own timeout). \
             Move timeout before retry for a single deadline across all attempts."
        );
    }

    if let (Some(r), Some(rl)) = (retry_pos, rate_limiter_pos)
        && rl > r
    {
        tracing::warn!(
            "ResiliencePipeline: rate_limiter is inside retry (rate-limited rejections trigger retries). \
             Move rate_limiter before retry to reject before entering the retry loop."
        );
    }
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// A composed resilience pipeline that applies multiple patterns in order.
///
/// Build via [`ResiliencePipeline::builder()`].
pub struct ResiliencePipeline<E: 'static> {
    steps: Arc<Vec<Step<E>>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Arc<dyn MetricsSink>,
}

impl<E: 'static> fmt::Debug for ResiliencePipeline<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResiliencePipeline")
            .field("steps", &self.steps.len())
            .finish_non_exhaustive()
    }
}

impl<E: Send + 'static> ResiliencePipeline<E> {
    /// Create a new builder.
    #[must_use]
    pub const fn builder() -> PipelineBuilder<E> {
        PipelineBuilder::new()
    }

    /// Execute `f` through all pipeline steps.
    ///
    /// # Errors
    ///
    /// Returns the appropriate `CallError` variant depending on which pipeline
    /// step fails (timeout, retry exhaustion, circuit open, bulkhead full, or operation error).
    pub async fn call<T, F, Fut>(&self, f: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        // Wrap the generic future factory in a single Box::pin adapter so the
        // internal pipeline machinery (which needs to erase the concrete Fut
        // type for Arc<F> sharing across retry iterations) only allocates
        // once per call instead of once per pipeline step.
        let boxed = move || -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> { Box::pin(f()) };
        execute_pipeline(
            Arc::clone(&self.steps),
            self.classifier.clone(),
            Arc::clone(&self.sink),
            Arc::new(boxed),
        )
        .await
    }

    /// Execute `f` through the pipeline with a fallback strategy.
    ///
    /// If the pipeline returns an error and the fallback's
    /// [`should_fallback`](crate::fallback::FallbackStrategy::should_fallback) returns true,
    /// the fallback strategy is invoked to recover.
    ///
    /// # Errors
    ///
    /// Returns the fallback's error if both the pipeline and fallback fail.
    pub async fn call_with_fallback<T, F, Fut>(
        &self,
        f: F,
        fallback: &dyn crate::fallback::FallbackStrategy<T, E>,
    ) -> Result<T, CallError<E>>
    where
        T: Send + Sync + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        match self.call(f).await {
            Ok(v) => Ok(v),
            Err(err) => {
                if fallback.should_fallback(&err) {
                    fallback.fallback(err).await
                } else {
                    Err(err)
                }
            },
        }
    }
}

async fn execute_pipeline<T, E, F>(
    steps: Arc<Vec<Step<E>>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Arc<dyn MetricsSink>,
    f: Arc<F>,
) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    run_operation_with_shells(&steps, classifier, sink, 0, f).await
}

/// Recursively apply pipeline steps (one `Box::pin` per Timeout/Retry shell),
/// then call the user function.
fn run_operation_with_shells<T, E, F>(
    steps: &Arc<Vec<Step<E>>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Arc<dyn MetricsSink>,
    idx: usize,
    f: Arc<F>,
) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    let steps = Arc::clone(steps);
    Box::pin(async move {
        if idx >= steps.len() {
            return f().await.map_err(CallError::Operation);
        }

        match &steps[idx] {
            Step::Timeout(d) => {
                let d = *d;
                tokio::time::timeout(
                    d,
                    run_operation_with_shells(
                        &steps,
                        classifier.clone(),
                        Arc::clone(&sink),
                        idx + 1,
                        f,
                    ),
                )
                .await
                .unwrap_or_else(|_| {
                    sink.record(ResilienceEvent::TimeoutElapsed { duration: d });
                    Err(CallError::Timeout(d))
                })
            },
            Step::Retry(config) => {
                run_retry_step(
                    config,
                    Arc::clone(&steps),
                    classifier,
                    Arc::clone(&sink),
                    idx,
                    f,
                )
                .await
            },
            Step::CircuitBreaker(cb) => {
                cb.try_acquire()?;
                let mut guard = ProbeGuard::new(cb);
                let result = run_operation_with_shells(
                    &steps,
                    classifier.clone(),
                    Arc::clone(&sink),
                    idx + 1,
                    Arc::clone(&f),
                )
                .await;
                guard.defuse();

                let outcome = classify_cb_outcome(&result, classifier.as_ref());
                cb.record_outcome(outcome);
                result
            },
            Step::Bulkhead(bh) => {
                let _permit = bh.acquire().await?;
                run_operation_with_shells(&steps, classifier, sink, idx + 1, f).await
            },
            Step::RateLimiter(check) => {
                if let Err(e) = check().await {
                    sink.record(ResilienceEvent::RateLimitExceeded);
                    return Err(CallError::RateLimited {
                        retry_after: e.retry_after(),
                    });
                }
                run_operation_with_shells(&steps, classifier, sink, idx + 1, f).await
            },
            Step::LoadShed(predicate) => {
                if predicate() {
                    sink.record(ResilienceEvent::LoadShed);
                    Err(CallError::LoadShed)
                } else {
                    run_operation_with_shells(&steps, classifier, sink, idx + 1, f).await
                }
            },
        }
    })
}

/// Classify the outcome of an inner pipeline result for the CB step.
///
/// When a classifier is available, operation errors are mapped via
/// `ErrorClass` → Outcome. Without a classifier, falls back to the
/// previous behavior (all operation errors = `Failure`).
fn classify_cb_outcome<T, E>(
    result: &Result<T, CallError<E>>,
    classifier: Option<&Arc<dyn ErrorClassifier<E>>>,
) -> Outcome {
    match result {
        Ok(_) => Outcome::Success,
        Err(CallError::Operation(e)) => {
            classifier.map_or(Outcome::Failure, |c| c.classify(e).into())
        },
        Err(CallError::RetriesExhausted { last, .. }) => {
            classifier.map_or(Outcome::Failure, |c| c.classify(last).into())
        },
        Err(CallError::Timeout(_)) => Outcome::Timeout,
        Err(_) => Outcome::Cancelled,
    }
}

/// Execute the Retry step of the pipeline.
#[allow(clippy::excessive_nesting)]
async fn run_retry_step<T, E, F>(
    config: &RetryConfig<E>,
    steps: Arc<Vec<Step<E>>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Arc<dyn MetricsSink>,
    idx: usize,
    f: Arc<F>,
) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    // `bail` captures the first non-operation error from the inner pipeline.
    // Once set, the retry predicate returns `false` (via `bail_check.is_none()`),
    // stopping retries immediately. It is never cleared because non-operation
    // errors (CircuitOpen, BulkheadFull, etc.) indicate the inner pipeline is
    // unreachable — retrying would hit the same structural error.
    let bail: Arc<Mutex<Option<CallError<E>>>> = Arc::new(Mutex::new(None));
    let config_classifier = config.classifier.clone();
    let pipeline_classifier = classifier.clone();
    let mut inner_config = RetryConfig::<Option<E>>::new_unchecked(config.max_attempts)
        .backoff(config.backoff.clone())
        .jitter(config.jitter.clone());
    inner_config.total_budget = config.total_budget;
    inner_config.sink = Arc::clone(&config.sink);
    inner_config.classifier = Some(Arc::new(FnClassifier::new(move |e: &Option<E>| {
        let Some(inner) = e else {
            return ErrorClass::Permanent;
        };
        config_classifier.as_ref().map_or_else(
            || {
                pipeline_classifier
                    .as_ref()
                    .map_or(ErrorClass::Transient, |classifier| {
                        classifier.classify(inner)
                    })
            },
            |classifier| classifier.classify(inner),
        )
    })));
    inner_config.on_retry = config.on_retry.as_ref().map(|notify| {
        let notify = Arc::clone(notify);
        Arc::new(move |e: &Option<E>, delay: Duration, attempt: u32| {
            if let Some(inner) = e {
                notify(inner, delay, attempt);
            }
        }) as Arc<dyn Fn(&Option<E>, Duration, u32) + Send + Sync>
    });

    let result = retry_with_inner(inner_config, {
        let steps = Arc::clone(&steps);
        let f = Arc::clone(&f);
        let bail = Arc::clone(&bail);
        let classifier = classifier.clone();
        let sink = Arc::clone(&sink);
        move || {
            let steps = Arc::clone(&steps);
            let f = Arc::clone(&f);
            let bail = Arc::clone(&bail);
            let classifier = classifier.clone();
            let sink = Arc::clone(&sink);
            Box::pin(async move {
                classify_inner(
                    run_operation_with_shells(&steps, classifier, sink, idx + 1, f).await,
                    &bail,
                )
            })
        }
    })
    .await;

    map_retry_result(result, &bail)
}

/// Classify an inner pipeline result for the retry layer.
///
/// `Ok` and `Operation` errors pass through; non-operation errors are
/// stashed in `bail` and signalled as `Err(None)` to stop retrying.
fn classify_inner<T, E>(
    result: Result<T, CallError<E>>,
    bail: &Arc<Mutex<Option<CallError<E>>>>,
) -> Result<T, Option<E>> {
    match result {
        Ok(v) => Ok(v),
        Err(CallError::Operation(e)) => Err(Some(e)),
        Err(other) => {
            *bail.lock() = Some(other);
            Err(None)
        },
    }
}

/// Map `CallError<Option<E>>` back to `CallError<E>` after retry.
fn map_retry_result<T, E: Send>(
    result: Result<T, CallError<Option<E>>>,
    bail: &Arc<Mutex<Option<CallError<E>>>>,
) -> Result<T, CallError<E>> {
    let take_bail = || bail.lock().take().unwrap_or(CallError::cancelled());

    match result {
        Ok(v) => Ok(v),
        Err(e) => Err(e.flat_map_inner(
            |opt| opt.map_or_else(take_bail, CallError::Operation),
            |attempts, opt| {
                opt.map_or_else(take_bail, |e| CallError::RetriesExhausted {
                    attempts,
                    last: e,
                })
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use super::*;
    use crate::{
        CallError, CircuitBreaker, RecordingSink, ResilienceEventKind, retry::BackoffConfig,
    };

    #[tokio::test]
    async fn pipeline_timeout_wraps_retry() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let pipeline = ResiliencePipeline::<&str>::builder()
            .timeout(Duration::from_secs(5))
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::from_millis(1))),
            )
            .build();

        let result = pipeline
            .call(move || {
                let c = c.clone();
                Box::pin(async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Err::<u32, &str>("fail")
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
    async fn pipeline_warns_on_bad_layer_order() {
        // timeout INSIDE retry is suboptimal — just verify build() succeeds
        let _pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(2)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::from_millis(1))),
            )
            .timeout(Duration::from_secs(1))
            .build();
    }

    #[tokio::test]
    async fn pipeline_returns_ok_on_success() {
        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn pipeline_retry_preserves_retry_if_predicate() {
        let attempts = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&attempts);

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(RetryConfig::new(3).unwrap().retry_if(|_: &&str| false))
            .build();

        let result = pipeline
            .call(move || {
                let seen = Arc::clone(&seen);
                Box::pin(async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Err::<u32, &str>("permanent")
                })
            })
            .await;

        assert!(matches!(result, Err(CallError::Operation("permanent"))));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pipeline_retry_preserves_retry_hooks() {
        let sink = RecordingSink::new();
        let notifications: Arc<StdMutex<Vec<(u32, Duration)>>> =
            Arc::new(StdMutex::new(Vec::new()));
        let seen_notifications = Arc::clone(&notifications);
        let attempts = Arc::new(AtomicU32::new(0));
        let seen_attempts = Arc::clone(&attempts);

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
                    .with_sink(sink.clone())
                    .on_retry(move |_err: &&str, delay: Duration, attempt: u32| {
                        seen_notifications.lock().unwrap().push((attempt, delay));
                    }),
            )
            .build();

        let result = pipeline
            .call(move || {
                let seen_attempts = Arc::clone(&seen_attempts);
                Box::pin(async move {
                    seen_attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<u32, &str>("transient")
                })
            })
            .await;

        assert!(matches!(
            result,
            Err(CallError::RetriesExhausted {
                attempts: 3,
                last: "transient",
            })
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 3);
        assert_eq!(notifications.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn pipeline_timeout_fires() {
        let pipeline = ResiliencePipeline::<&str>::builder()
            .timeout(Duration::from_millis(10))
            .build();

        let result = pipeline
            .call(|| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<u32, &str>(42)
                })
            })
            .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
    }

    #[tokio::test]
    async fn pipeline_rate_limiter_inside_cb_does_not_panic() {
        use crate::circuit_breaker::CircuitBreakerConfig;

        let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default()).unwrap());

        // Rate limiter that always rejects
        let rl: RateLimitCheck = Arc::new(|| Box::pin(async { Err(CallError::rate_limited()) }));

        let pipeline = ResiliencePipeline::<&str>::builder()
            .circuit_breaker(cb)
            .rate_limiter(rl)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        // Should return RateLimited, not panic
        assert!(matches!(result, Err(CallError::RateLimited { .. })));
    }

    #[tokio::test]
    async fn pipeline_with_sink_emits_timeout_event() {
        let sink = RecordingSink::new();
        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .timeout(Duration::from_millis(10))
            .build();

        let result = pipeline
            .call(|| {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok::<u32, &str>(42)
                })
            })
            .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
        assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);
    }

    #[tokio::test]
    async fn pipeline_with_sink_emits_rate_limit_event() {
        let sink = RecordingSink::new();
        let rate_limiter: RateLimitCheck = Arc::new(|| {
            Box::pin(async { Err(CallError::rate_limited_after(Duration::from_secs(2))) })
        });
        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .rate_limiter(rate_limiter)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        assert!(matches!(
            result,
            Err(CallError::RateLimited {
                retry_after: Some(duration),
            }) if duration == Duration::from_secs(2)
        ));
        assert_eq!(sink.count(ResilienceEventKind::RateLimitExceeded), 1);
    }

    #[tokio::test]
    async fn pipeline_with_sink_emits_load_shed_event() {
        let sink = RecordingSink::new();
        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .load_shed(Arc::new(|| true))
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        assert!(matches!(result, Err(CallError::LoadShed)));
        assert_eq!(sink.count(ResilienceEventKind::LoadShed), 1);
    }

    #[tokio::test]
    async fn pipeline_cb_half_open_allows_single_probe() {
        use crate::{
            circuit_breaker::{CircuitBreakerConfig, Outcome},
            sink::CircuitState,
        };

        let cb = Arc::new(
            CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 2,
                reset_timeout: Duration::from_millis(50),
                max_half_open_operations: 1,
                min_operations: 1,
                count_timeouts_as_failures: true,
                ..Default::default()
            })
            .unwrap(),
        );

        // Trip the breaker
        cb.record_outcome(Outcome::Failure);
        cb.record_outcome(Outcome::Failure);
        assert_eq!(cb.circuit_state(), CircuitState::Open);

        // Wait for reset timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Pipeline should succeed through HalfOpen → Closed
        let pipeline = ResiliencePipeline::<&str>::builder()
            .circuit_breaker(Arc::clone(&cb))
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(cb.circuit_state(), CircuitState::Closed);
    }

    #[tokio::test]
    async fn pipeline_bulkhead_takes_single_permit() {
        let bh = Arc::new(
            Bulkhead::new(crate::BulkheadConfig {
                max_concurrency: 2,
                queue_size: 1,
                timeout: None,
            })
            .unwrap(),
        );

        let pipeline = ResiliencePipeline::<&str>::builder()
            .bulkhead(Arc::clone(&bh))
            .build();

        // After pipeline.call completes, the permit should be released
        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;
        assert_eq!(result.unwrap(), 42);

        // Both permits should be available again
        assert_eq!(bh.available_permits(), 2);
    }

    #[tokio::test]
    async fn pipeline_call_with_fallback_recovers() {
        use crate::fallback::ValueFallback;

        let pipeline = ResiliencePipeline::<&str>::builder()
            .timeout(Duration::from_millis(10))
            .build();

        let fallback = ValueFallback::new(99u32);

        let result = pipeline
            .call_with_fallback(
                || {
                    Box::pin(async {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Ok::<u32, &str>(42)
                    })
                },
                &fallback,
            )
            .await;

        assert_eq!(result.unwrap(), 99);
    }

    #[tokio::test]
    async fn pipeline_call_with_fallback_passes_through_on_success() {
        use crate::fallback::ValueFallback;

        let pipeline = ResiliencePipeline::<&str>::builder().build();
        let fallback = ValueFallback::new(0u32);

        let result = pipeline
            .call_with_fallback(|| Box::pin(async { Ok::<u32, &str>(42) }), &fallback)
            .await;

        assert_eq!(result.unwrap(), 42);
    }
}
