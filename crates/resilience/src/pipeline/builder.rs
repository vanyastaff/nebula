//! Pipeline builder — step collection, ordering validation, and construction.

use std::{fmt, sync::Arc, time::Duration};

use crate::{
    bulkhead::Bulkhead,
    circuit_breaker::CircuitBreaker,
    classifier::ErrorClassifier,
    events::{EventScope, EventSink, NoopSink},
    rate_limiter::ErasedRateLimiter,
    retry::RetryConfig,
};

use super::executor::ResiliencePipeline;
use super::{LoadShedPredicate, RateLimitCheck, RetryHintFn, Step};

/// Builder for [`ResiliencePipeline`].
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::{PipelineBuilder, ResiliencePipeline};
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let builder: PipelineBuilder<&str> =
///     ResiliencePipeline::<&str>::builder().timeout(Duration::from_secs(1));
///
/// let pipeline = builder.build();
/// // The operation returns `Ok` unconditionally, so this cannot fail.
/// let value = pipeline
///     .call(|| Box::pin(async { Ok::<_, &str>(7u32) }))
///     .await
///     .expect("the operation succeeds by construction");
/// assert_eq!(value, 7);
/// # Ok(())
/// # }
/// ```
pub struct PipelineBuilder<E: 'static> {
    pub(super) steps: Vec<Step<E>>,
    pub(super) classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    pub(super) sink: Option<Arc<dyn EventSink>>,
    pub(super) retry_hint: Option<RetryHintFn<E>>,
    pub(super) scope: EventScope,
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
            retry_hint: None,
            scope: EventScope::empty(),
        }
    }

    /// Set an [`ErrorClassifier`] for the pipeline.
    ///
    /// When set, the circuit breaker step uses
    /// [`call_with_classifier`](CircuitBreaker::call_with_classifier) instead of
    /// [`call`](CircuitBreaker::call). The retry step combines this with any classifier on
    /// [`RetryConfig`] (per-retry classifier wins for retry
    /// decisions; the pipeline classifier still applies to the circuit breaker).
    ///
    /// Without a **pipeline** classifier, operation errors that reach the circuit breaker are
    /// all counted as [`Failure`](crate::circuit_breaker::Outcome::Failure) for CB state. The
    /// retry step, when neither a pipeline nor a per-retry classifier is set, treats operation
    /// errors as permanent. To retry caller errors, set
    /// [`classify_errors()`](Self::classify_errors) and/or per-retry
    /// [`retry_if`](crate::retry::RetryConfig::retry_if) /
    /// [`with_classifier`](crate::retry::RetryConfig::with_classifier), ideally only for
    /// idempotent operations.
    #[must_use]
    pub fn classifier(mut self, classifier: Arc<dyn ErrorClassifier<E>>) -> Self {
        self.classifier = Some(classifier);
        self
    }

    /// Inject a pipeline-wide metrics sink.
    ///
    /// The sink receives events emitted by pipeline-managed wrappers and is used
    /// for retry attempts configured through this builder. Circuit breakers and
    /// bulkheads passed in as pre-built `Arc`s keep their own sinks.
    #[must_use]
    pub fn with_sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.sink = Some(Arc::new(sink));
        self
    }

    /// Attach workflow/resource scope to pipeline completion events.
    ///
    /// Keep values low-cardinality when forwarding these events to metrics.
    #[must_use]
    pub fn scope(mut self, scope: EventScope) -> Self {
        self.scope = scope;
        self
    }

    /// Set a retry-delay hint extractor for operation errors.
    ///
    /// The returned duration is used as a minimum retry delay, merged with the
    /// configured [`BackoffConfig`](crate::retry::BackoffConfig). This is useful
    /// when a custom classifier is used and operation errors carry retry-after
    /// metadata outside [`Classify`](nebula_error::Classify).
    #[must_use]
    pub fn retry_hint<F>(mut self, hint: F) -> Self
    where
        F: Fn(&E) -> Option<Duration> + Send + Sync + 'static,
    {
        self.retry_hint = Some(Arc::new(hint));
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
    /// Pipeline retry uses the configured [`BackoffConfig`](crate::retry::BackoffConfig)
    /// and merges in retry-delay floors from [`retry_hint`](Self::retry_hint).
    /// When [`classify_errors`](Self::classify_errors) is used, `E: Classify`
    /// retry-after hints are wired automatically.
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

    /// Add a rate limiter step using a concrete [`RateLimiter`](crate::RateLimiter) implementation.
    ///
    /// The `Arc<RL>` is required because the rate limiter must be shared across
    /// potentially multiple retry attempts and concurrent pipeline invocations.
    /// The pipeline internally clones the `Arc` into an async closure that must
    /// be `Send + Sync + 'static`, so shared ownership via `Arc` is the only
    /// way to satisfy those bounds without copying or locking the entire limiter.
    ///
    /// This is the ergonomic way to add rate limiting — it handles the closure
    /// bridging automatically:
    ///
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use nebula_resilience::{ResiliencePipeline, rate_limiter::TokenBucket};
    ///
    /// let rl = Arc::new(TokenBucket::new(100, 10.0).unwrap());
    /// let pipeline = ResiliencePipeline::<String>::builder()
    ///     .rate_limiter_from(rl)
    ///     .build();
    /// ```
    ///
    /// If you need a custom bridging closure (e.g., wrapping a non-`RateLimiter`
    /// type), use [`rate_limiter`](Self::rate_limiter) directly.
    #[must_use]
    pub fn rate_limiter_from<RL: crate::RateLimiter + 'static>(self, rl: Arc<RL>) -> Self {
        let check: RateLimitCheck = Arc::new(move || {
            let rl = Arc::clone(&rl);
            Box::pin(async move { rl.acquire().await })
        });
        self.rate_limiter(check)
    }

    /// Add a rate limiter step from an object-safe rate limiter registry entry.
    ///
    /// Use this when policies are selected dynamically and stored as
    /// `Arc<dyn ErasedRateLimiter>`, for example tenant- or resource-scoped
    /// limiters loaded from runtime configuration. Prefer
    /// [`rate_limiter_from`](Self::rate_limiter_from) when the concrete limiter
    /// type is known at compile time.
    ///
    /// ```rust,no_run
    /// use std::{sync::Arc, time::Duration};
    ///
    /// use nebula_resilience::{
    ///     ErasedRateLimiter, ResiliencePipeline,
    ///     rate_limiter::{SlidingWindow, TokenBucket},
    /// };
    ///
    /// let registry: Vec<Arc<dyn ErasedRateLimiter>> = vec![
    ///     Arc::new(TokenBucket::new(100, 10.0).unwrap()),
    ///     Arc::new(SlidingWindow::new(Duration::from_secs(60), 100).unwrap()),
    /// ];
    ///
    /// let pipeline = ResiliencePipeline::<String>::builder()
    ///     .rate_limiter_erased(Arc::clone(&registry[0]))
    ///     .build();
    /// # let _ = pipeline;
    /// ```
    #[must_use]
    pub fn rate_limiter_erased(self, rl: Arc<dyn ErasedRateLimiter>) -> Self {
        let check: RateLimitCheck = Arc::new(move || {
            let rl = Arc::clone(&rl);
            Box::pin(async move { rl.acquire_boxed().await })
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
        warn_on_suboptimal_order(&self.steps);
        self.build_inner()
    }

    /// Builds the pipeline only if steps are already in the recommended order.
    ///
    /// This is intended for config/schema-driven construction where warnings are
    /// too easy to miss. Use [`build_sorted`](Self::build_sorted)
    /// when policy declarations may arrive in arbitrary order and sorting is acceptable.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if a later step should be outside an earlier one.
    pub fn try_build(self) -> Result<ResiliencePipeline<E>, crate::ConfigError> {
        require_recommended_order(&self.steps)?;
        Ok(self.build_inner())
    }

    /// Build the pipeline after sorting layers into the recommended order.
    ///
    /// This preserves insertion order among steps of the same kind and orders
    /// different kinds as: `load_shed -> rate_limiter -> timeout -> retry ->
    /// circuit_breaker -> bulkhead`.
    #[must_use]
    pub fn build_sorted(mut self) -> ResiliencePipeline<E> {
        self.steps.sort_by_key(step_rank);
        self.build_inner()
    }

    fn build_inner(self) -> ResiliencePipeline<E> {
        let sink_overrides_steps = self.sink.is_some();
        ResiliencePipeline {
            steps: Arc::new(self.steps),
            classifier: self.classifier,
            sink: self.sink.unwrap_or_else(|| Arc::new(NoopSink)),
            sink_overrides_steps,
            retry_hint: self.retry_hint,
            scope: self.scope,
        }
    }
}

/// Convenience: set [`NebulaClassifier`](crate::classifier::NebulaClassifier)
/// when `E` implements [`Classify`](nebula_error::Classify).
impl<E: nebula_error::Classify + Send + Sync + 'static> PipelineBuilder<E> {
    /// Use [`NebulaClassifier`](crate::classifier::NebulaClassifier) to automatically
    /// map [`ErrorCategory`](nebula_error::ErrorCategory) to
    /// [`ErrorClass`].
    ///
    /// This is the recommended default for pipelines where `E: Classify`.
    #[must_use]
    pub fn classify_errors(mut self) -> Self {
        self.classifier = Some(Arc::new(crate::classifier::NebulaClassifier));
        if self.retry_hint.is_none() {
            self.retry_hint = Some(Arc::new(|e: &E| e.retry_hint().and_then(|h| h.after)));
        }
        self
    }
}

const fn step_rank<E>(step: &Step<E>) -> u8 {
    match step {
        Step::LoadShed(_) => 0,
        Step::RateLimiter(_) => 1,
        Step::Timeout(_) => 2,
        Step::Retry(_) => 3,
        Step::CircuitBreaker(_) => 4,
        Step::Bulkhead(_) => 5,
    }
}

const fn step_name<E>(step: &Step<E>) -> &'static str {
    match step {
        Step::LoadShed(_) => "load_shed",
        Step::RateLimiter(_) => "rate_limiter",
        Step::Timeout(_) => "timeout",
        Step::Retry(_) => "retry",
        Step::CircuitBreaker(_) => "circuit_breaker",
        Step::Bulkhead(_) => "bulkhead",
    }
}

fn require_recommended_order<E>(steps: &[Step<E>]) -> Result<(), crate::ConfigError> {
    let mut highest_rank = 0u8;
    let mut highest_name = None;

    for step in steps {
        let rank = step_rank(step);
        if rank < highest_rank {
            let earlier = highest_name.unwrap_or("earlier policy");
            return Err(crate::ConfigError::new(
                "pipeline_order",
                format!(
                    "{} must be added before {}; use build_sorted() to sort config-driven pipelines",
                    step_name(step),
                    earlier
                ),
            ));
        }
        if rank > highest_rank {
            highest_rank = rank;
            highest_name = Some(step_name(step));
        }
    }

    Ok(())
}

fn warn_on_suboptimal_order<E>(steps: &[Step<E>]) {
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
            "ResiliencePipeline: rate_limiter is inside retry (rate-limited rejections may be retried per attempt). \
             Move rate_limiter before retry to reject once before entering the retry loop."
        );
    }
}
