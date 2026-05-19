//! `ResiliencePipeline` — compose multiple resilience patterns into a single call chain.
//!
//! Recommended layer order (outermost → innermost):
//! `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`
//!
//! Layers are applied in the order added: first added = outermost.
//!
//! # Examples
//!
//! ```rust
//! use std::time::Duration;
//!
//! use nebula_resilience::{
//!     ResiliencePipeline,
//!     retry::{BackoffConfig, RetryConfig},
//! };
//!
//! # #[tokio::main]
//! # async fn main() {
//! let pipeline = ResiliencePipeline::<&str>::builder()
//!     .timeout(Duration::from_secs(2))
//!     .retry(
//!         RetryConfig::new(3)
//!             .expect("max_attempts >= 1")
//!             .backoff(BackoffConfig::Fixed(Duration::from_millis(10))),
//!     )
//!     .build();
//!
//! let value = pipeline
//!     .call(|| Box::pin(async { Ok::<_, &str>(42u32) }))
//!     .await
//!     .unwrap();
//! assert_eq!(value, 42);
//! # }
//! ```

use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use crate::{
    CallError, PolicyContext,
    bulkhead::Bulkhead,
    cancellation::CancellationContext,
    circuit_breaker::{CircuitBreaker, Outcome, ProbeGuard},
    classifier::{ErrorClass, ErrorClassifier, FnClassifier},
    rate_limiter::{ErasedRateLimiter, map_acquire_error},
    retry::{RetryConfig, retry_with},
    sink::{MetricsSink, NoopSink, PipelineOutcome, PolicyScope, ResilienceEvent},
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
// Retry for the back-off loop with `retry_with`.

/// Async predicate for rate limiting — returns `Ok(())` or `Err(CallError::RateLimited)`.
pub type RateLimitCheck =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), CallError<()>>> + Send>> + Send + Sync>;

/// Predicate for load shedding — returns `true` to shed the request.
pub type LoadShedPredicate = Arc<dyn Fn() -> bool + Send + Sync>;

type RetryHintFn<E> = Arc<dyn Fn(&E) -> Option<Duration> + Send + Sync>;

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
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::{PipelineBuilder, ResiliencePipeline};
///
/// # #[tokio::main]
/// # async fn main() {
/// let builder: PipelineBuilder<&str> =
///     ResiliencePipeline::<&str>::builder().timeout(Duration::from_secs(1));
///
/// let pipeline = builder.build();
/// let value = pipeline
///     .call(|| Box::pin(async { Ok::<_, &str>(7u32) }))
///     .await
///     .unwrap();
/// assert_eq!(value, 7);
/// # }
/// ```
pub struct PipelineBuilder<E: 'static> {
    steps: Vec<Step<E>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Option<Arc<dyn MetricsSink>>,
    retry_hint: Option<RetryHintFn<E>>,
    scope: PolicyScope,
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
            scope: PolicyScope::empty(),
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
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Some(Arc::new(sink));
        self
    }

    /// Attach workflow/resource scope to pipeline completion events.
    ///
    /// Keep values low-cardinality when forwarding these events to metrics.
    #[must_use]
    pub fn scope(mut self, scope: PolicyScope) -> Self {
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
        validate_order(&self.steps);
        self.build_inner()
    }

    /// Build the pipeline only if steps are already in the recommended order.
    ///
    /// This is intended for config/schema-driven construction where warnings are
    /// too easy to miss. Use [`build_recommended_order`](Self::build_recommended_order)
    /// when policy declarations may arrive in arbitrary order and sorting is acceptable.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if a later step should be outside an earlier one.
    pub fn build_checked(self) -> Result<ResiliencePipeline<E>, crate::ConfigError> {
        validate_recommended_order(&self.steps)?;
        Ok(self.build_inner())
    }

    /// Build the pipeline after sorting layers into the recommended order.
    ///
    /// This preserves insertion order among steps of the same kind and orders
    /// different kinds as: `load_shed -> rate_limiter -> timeout -> retry ->
    /// circuit_breaker -> bulkhead`.
    #[must_use]
    pub fn build_recommended_order(mut self) -> ResiliencePipeline<E> {
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

fn validate_recommended_order<E>(steps: &[Step<E>]) -> Result<(), crate::ConfigError> {
    let mut highest_rank = 0u8;
    let mut highest_name = None;

    for step in steps {
        let rank = step_rank(step);
        if rank < highest_rank {
            let earlier = highest_name.unwrap_or("earlier policy");
            return Err(crate::ConfigError::new(
                "pipeline_order",
                format!(
                    "{} must be added before {}; use build_recommended_order() to sort config-driven pipelines",
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
            "ResiliencePipeline: rate_limiter is inside retry (rate-limited rejections may be retried per attempt). \
             Move rate_limiter before retry to reject once before entering the retry loop."
        );
    }
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// A composed resilience pipeline that applies multiple patterns in order.
///
/// Build via [`ResiliencePipeline::builder()`].
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::ResiliencePipeline;
///
/// # #[tokio::main]
/// # async fn main() {
/// let pipeline = ResiliencePipeline::<&str>::builder()
///     .timeout(Duration::from_millis(50))
///     .build();
///
/// let value = pipeline
///     .call(|| Box::pin(async { Ok::<_, &str>("ok") }))
///     .await
///     .unwrap();
/// assert_eq!(value, "ok");
/// # }
/// ```
pub struct ResiliencePipeline<E: 'static> {
    steps: Arc<Vec<Step<E>>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Arc<dyn MetricsSink>,
    sink_overrides_steps: bool,
    retry_hint: Option<RetryHintFn<E>>,
    scope: PolicyScope,
}

struct PipelineRunContext<E: 'static> {
    steps: Arc<Vec<Step<E>>>,
    classifier: Option<Arc<dyn ErrorClassifier<E>>>,
    sink: Arc<dyn MetricsSink>,
    sink_overrides_steps: bool,
    retry_hint: Option<RetryHintFn<E>>,
    cancellation: Option<CancellationContext>,
}

impl<E: 'static> Clone for PipelineRunContext<E> {
    fn clone(&self) -> Self {
        Self {
            steps: Arc::clone(&self.steps),
            classifier: self.classifier.clone(),
            sink: Arc::clone(&self.sink),
            sink_overrides_steps: self.sink_overrides_steps,
            retry_hint: self.retry_hint.clone(),
            cancellation: self.cancellation.clone(),
        }
    }
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
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await`. Every pipeline
    /// step's bookkeeping is stack-local or a drop guard — bulkhead permits
    /// and circuit-breaker probe slots are released on drop — so no
    /// crate-owned state is left partially mutated, and the pipeline does
    /// not detach work via `spawn`. Whether a *partially executed* operation
    /// is safe to abandon is the supplied operation's own contract.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::time::Duration;
    ///
    /// use nebula_resilience::{CallError, ResiliencePipeline};
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let pipeline = ResiliencePipeline::<&str>::builder()
    ///     .timeout(Duration::from_millis(20))
    ///     .build();
    ///
    /// // The operation never finishes inside the budget, so the pipeline returns Timeout.
    /// let err: CallError<&str> = pipeline
    ///     .call(|| {
    ///         Box::pin(async {
    ///             tokio::time::sleep(Duration::from_millis(200)).await;
    ///             Ok::<u32, &str>(42)
    ///         })
    ///     })
    ///     .await
    ///     .unwrap_err();
    /// assert!(matches!(err, CallError::Timeout(_)));
    /// # }
    /// ```
    pub async fn call<T, F, Fut>(&self, f: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        let result = self.call_inner(None, f).await;
        self.record_pipeline_completed(match &result {
            Ok(_) => PipelineOutcome::Success,
            Err(err) => PipelineOutcome::Failure { error: err.kind() },
        });
        result
    }

    /// Execute `f` through all pipeline steps, stopping promptly when `cancellation`
    /// is cancelled.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if cancellation fires before the pipeline
    /// completes, or the normal pipeline error otherwise.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await`. Every pipeline
    /// step's bookkeeping is stack-local or a drop guard — bulkhead permits
    /// and circuit-breaker probe slots are released on drop — so no
    /// crate-owned state is left partially mutated, and the pipeline does
    /// not detach work via `spawn`. Whether a *partially executed* operation
    /// is safe to abandon is the supplied operation's own contract.
    pub async fn call_with_context<T, F, Fut>(
        &self,
        cancellation: &CancellationContext,
        f: F,
    ) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        let result = self.call_inner(Some(cancellation.clone()), f).await;
        self.record_pipeline_completed(match &result {
            Ok(_) => PipelineOutcome::Success,
            Err(err) => PipelineOutcome::Failure { error: err.kind() },
        });
        result
    }

    /// Execute `f` through all pipeline steps using a shared policy context.
    ///
    /// `PolicyContext` groups cancellation, deadline, and observability scope so
    /// a workflow runtime can pass one execution contract through the policy
    /// stack. If the context has a deadline, it bounds the whole pipeline call.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline expires, or the normal
    /// pipeline error otherwise.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await`. Every pipeline
    /// step's bookkeeping is stack-local or a drop guard — bulkhead permits
    /// and circuit-breaker probe slots are released on drop — so no
    /// crate-owned state is left partially mutated, and the pipeline does
    /// not detach work via `spawn`. Whether a *partially executed* operation
    /// is safe to abandon is the supplied operation's own contract.
    pub async fn call_with_policy_context<T, F, Fut>(
        &self,
        context: &PolicyContext,
        f: F,
    ) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        let result = self
            .run_with_policy_deadline(context, self.call_inner(context.cancellation_cloned(), f))
            .await;
        self.record_pipeline_completed_for_scope(
            self.effective_scope(Some(context)),
            match &result {
                Ok(_) => PipelineOutcome::Success,
                Err(err) => PipelineOutcome::Failure { error: err.kind() },
            },
        );
        result
    }

    async fn call_inner<T, F, Fut>(
        &self,
        cancellation: Option<CancellationContext>,
        f: F,
    ) -> Result<T, CallError<E>>
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
            self.sink_overrides_steps,
            self.retry_hint.clone(),
            cancellation,
            Arc::new(boxed),
        )
        .await
    }

    fn record_pipeline_completed(&self, outcome: PipelineOutcome) {
        if !self.sink_overrides_steps {
            return;
        }
        self.record_pipeline_completed_for_scope(self.scope.clone(), outcome);
    }

    fn record_pipeline_completed_for_scope(&self, scope: PolicyScope, outcome: PipelineOutcome) {
        if !self.sink_overrides_steps {
            return;
        }
        self.sink
            .record(ResilienceEvent::PipelineCompleted { scope, outcome });
    }

    fn effective_scope(&self, context: Option<&PolicyContext>) -> PolicyScope {
        context.map_or_else(
            || self.scope.clone(),
            |context| {
                if context.scope().is_empty() {
                    self.scope.clone()
                } else {
                    context.scope().clone()
                }
            },
        )
    }

    async fn run_with_policy_deadline<T, Fut>(
        &self,
        context: &PolicyContext,
        future: Fut,
    ) -> Result<T, CallError<E>>
    where
        Fut: Future<Output = Result<T, CallError<E>>> + Send,
    {
        if let Some(deadline) = context.deadline() {
            match deadline.timeout(future).await {
                Ok(result) => result,
                Err(err) => {
                    self.sink.record(ResilienceEvent::TimeoutElapsed {
                        duration: deadline.budget(),
                    });
                    Err(err)
                },
            }
        } else {
            future.await
        }
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
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await`. Every pipeline
    /// step's bookkeeping is stack-local or a drop guard — bulkhead permits
    /// and circuit-breaker probe slots are released on drop — so no
    /// crate-owned state is left partially mutated, and the pipeline does
    /// not detach work via `spawn`. Whether a *partially executed* operation
    /// is safe to abandon is the supplied operation's own contract.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use std::time::Duration;
    ///
    /// use nebula_resilience::{ResiliencePipeline, fallback::ValueFallback};
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let pipeline = ResiliencePipeline::<&str>::builder()
    ///     .timeout(Duration::from_millis(10))
    ///     .build();
    /// let fallback = ValueFallback::new(99u32);
    ///
    /// // The pipeline times out, so the fallback value is returned instead.
    /// let value = pipeline
    ///     .call_with_fallback(
    ///         || {
    ///             Box::pin(async {
    ///                 tokio::time::sleep(Duration::from_millis(50)).await;
    ///                 Ok::<u32, &str>(42)
    ///             })
    ///         },
    ///         &fallback,
    ///     )
    ///     .await
    ///     .unwrap();
    /// assert_eq!(value, 99);
    /// # }
    /// ```
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
        self.call_with_fallback_inner(None, self.scope.clone(), f, fallback)
            .await
    }

    /// Execute `f` through the pipeline with both cancellation and fallback.
    ///
    /// Cancellation wins over fallback: if the context is cancelled before or
    /// during fallback execution, the method returns [`CallError::Cancelled`].
    /// This prevents engine shutdown from being reported as a successful
    /// fallback recovery.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if cancellation fires before the
    /// pipeline/fallback completes, or the normal pipeline/fallback error
    /// otherwise.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await`. Every pipeline
    /// step's bookkeeping is stack-local or a drop guard — bulkhead permits
    /// and circuit-breaker probe slots are released on drop — so no
    /// crate-owned state is left partially mutated, and the pipeline does
    /// not detach work via `spawn`. Whether a *partially executed* operation
    /// is safe to abandon is the supplied operation's own contract.
    pub async fn call_with_context_and_fallback<T, F, Fut>(
        &self,
        cancellation: &CancellationContext,
        f: F,
        fallback: &dyn crate::fallback::FallbackStrategy<T, E>,
    ) -> Result<T, CallError<E>>
    where
        T: Send + Sync + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        self.call_with_fallback_inner(Some(cancellation.clone()), self.scope.clone(), f, fallback)
            .await
    }

    /// Execute `f` through the pipeline with shared context and fallback.
    ///
    /// This is the most explicit workflow-runtime entry point: cancellation,
    /// deadline, scope, primary pipeline execution, and fallback recovery all
    /// share one context. Cancellation wins over fallback, and the context
    /// deadline bounds both the primary path and fallback path.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline expires, or the normal
    /// pipeline/fallback error otherwise.
    ///
    /// # Cancel safety
    ///
    /// Cancel-safe with respect to this crate: dropping the returned future
    /// drops the in-flight operation at its current `.await`. Every pipeline
    /// step's bookkeeping is stack-local or a drop guard — bulkhead permits
    /// and circuit-breaker probe slots are released on drop — so no
    /// crate-owned state is left partially mutated, and the pipeline does
    /// not detach work via `spawn`. Whether a *partially executed* operation
    /// is safe to abandon is the supplied operation's own contract.
    pub async fn call_with_policy_context_and_fallback<T, F, Fut>(
        &self,
        context: &PolicyContext,
        f: F,
        fallback: &dyn crate::fallback::FallbackStrategy<T, E>,
    ) -> Result<T, CallError<E>>
    where
        T: Send + Sync + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        let scope = self.effective_scope(Some(context));
        let future = self.call_with_fallback_inner(
            context.cancellation_cloned(),
            scope.clone(),
            f,
            fallback,
        );

        if let Some(deadline) = context.deadline() {
            match deadline.timeout(future).await {
                Ok(result) => result,
                Err(err) => {
                    self.sink.record(ResilienceEvent::TimeoutElapsed {
                        duration: deadline.budget(),
                    });
                    self.record_pipeline_completed_for_scope(
                        scope,
                        PipelineOutcome::Failure { error: err.kind() },
                    );
                    Err(err)
                },
            }
        } else {
            future.await
        }
    }

    async fn call_with_fallback_inner<T, F, Fut>(
        &self,
        cancellation: Option<CancellationContext>,
        completion_scope: PolicyScope,
        f: F,
        fallback: &dyn crate::fallback::FallbackStrategy<T, E>,
    ) -> Result<T, CallError<E>>
    where
        T: Send + Sync + 'static,
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
    {
        match self.call_inner(cancellation.clone(), f).await {
            Ok(v) => {
                self.record_pipeline_completed_for_scope(
                    completion_scope,
                    PipelineOutcome::Success,
                );
                Ok(v)
            },
            Err(err) => {
                let primary_error = err.kind();
                if matches!(err, CallError::Cancelled { .. }) {
                    self.record_pipeline_completed_for_scope(
                        completion_scope,
                        PipelineOutcome::Failure {
                            error: primary_error,
                        },
                    );
                    return Err(err);
                }
                if let Some(cancellation) = &cancellation
                    && cancellation.is_cancelled()
                {
                    let cancelled = cancellation.cancelled_error();
                    self.record_pipeline_completed_for_scope(
                        completion_scope,
                        PipelineOutcome::Failure {
                            error: cancelled.kind(),
                        },
                    );
                    return Err(cancelled);
                }
                if fallback.should_fallback(&err) {
                    self.sink
                        .record(ResilienceEvent::FallbackAttempted { primary_error });
                    let result = if let Some(cancellation) = cancellation.clone() {
                        tokio::select! {
                            result = fallback.recover(err) => result,
                            () = cancellation.token().cancelled() => Err(cancellation.cancelled_error()),
                        }
                    } else {
                        fallback.recover(err).await
                    };
                    let pipeline_outcome = match &result {
                        Ok(_) => {
                            self.sink
                                .record(ResilienceEvent::FallbackSucceeded { primary_error });
                            PipelineOutcome::FallbackSucceeded { primary_error }
                        },
                        Err(fallback_error) => {
                            self.sink.record(ResilienceEvent::FallbackFailed {
                                primary_error,
                                fallback_error: fallback_error.kind(),
                            });
                            PipelineOutcome::FallbackFailed {
                                primary_error,
                                fallback_error: fallback_error.kind(),
                            }
                        },
                    };
                    self.record_pipeline_completed_for_scope(completion_scope, pipeline_outcome);
                    result
                } else {
                    self.record_pipeline_completed_for_scope(
                        completion_scope,
                        PipelineOutcome::Failure {
                            error: primary_error,
                        },
                    );
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
    sink_overrides_steps: bool,
    retry_hint: Option<RetryHintFn<E>>,
    cancellation: Option<CancellationContext>,
    f: Arc<F>,
) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    let ctx = PipelineRunContext {
        steps,
        classifier,
        sink,
        sink_overrides_steps,
        retry_hint,
        cancellation,
    };
    run_operation_with_shells(ctx, 0, f).await
}

/// Recursively apply pipeline steps (one `Box::pin` per Timeout/Retry shell),
/// then call the user function.
fn run_operation_with_shells<T, E, F>(
    ctx: PipelineRunContext<E>,
    idx: usize,
    f: Arc<F>,
) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    let steps = Arc::clone(&ctx.steps);
    Box::pin(async move {
        if let Some(cancellation) = &ctx.cancellation
            && cancellation.is_cancelled()
        {
            return Err(cancellation.cancelled_error());
        }

        if idx >= steps.len() {
            return if let Some(cancellation) = ctx.cancellation.clone() {
                tokio::select! {
                    result = f() => result.map_err(CallError::Operation),
                    () = cancellation.token().cancelled() => Err(cancellation.cancelled_error()),
                }
            } else {
                f().await.map_err(CallError::Operation)
            };
        }

        match &steps[idx] {
            Step::Timeout(d) => {
                let d = *d;
                let inner = run_operation_with_shells(ctx.clone(), idx + 1, f);
                if let Some(cancellation) = ctx.cancellation.clone() {
                    tokio::select! {
                        result = tokio::time::timeout(d, inner) => {
                            result.unwrap_or_else(|_| {
                                ctx.sink
                                    .record(ResilienceEvent::TimeoutElapsed { duration: d });
                                Err(CallError::Timeout(d))
                            })
                        },
                        () = cancellation.token().cancelled() => Err(cancellation.cancelled_error()),
                    }
                } else {
                    tokio::time::timeout(d, inner).await.unwrap_or_else(|_| {
                        ctx.sink
                            .record(ResilienceEvent::TimeoutElapsed { duration: d });
                        Err(CallError::Timeout(d))
                    })
                }
            },
            Step::Retry(config) => run_retry_step(config, ctx, idx, f).await,
            Step::CircuitBreaker(cb) => {
                cb.try_acquire()?;

                let mut guard = ProbeGuard::new(cb);
                let start = cb.tracks_slow_calls().then(|| cb.clock_now());
                let result = run_operation_with_shells(ctx.clone(), idx + 1, Arc::clone(&f)).await;
                let duration = start.map(|start| cb.clock_now().duration_since(start));
                guard.defuse();

                let outcome = classify_cb_outcome(cb, &result, ctx.classifier.as_ref(), duration);
                cb.record_outcome(outcome);
                result
            },
            Step::Bulkhead(bh) => {
                let acquire = bh.acquire();
                let _permit = if let Some(cancellation) = ctx.cancellation.clone() {
                    tokio::select! {
                        result = acquire => result?,
                        () = cancellation.token().cancelled() => return Err(cancellation.cancelled_error()),
                    }
                } else {
                    acquire.await?
                };
                run_operation_with_shells(ctx, idx + 1, f).await
            },
            Step::RateLimiter(check) => {
                let check_result = if let Some(cancellation) = ctx.cancellation.clone() {
                    tokio::select! {
                        result = check() => result,
                        () = cancellation.token().cancelled() => return Err(cancellation.cancelled_error()),
                    }
                } else {
                    check().await
                };
                match check_result {
                    Ok(()) => {},
                    Err(CallError::RateLimited { retry_after }) => {
                        ctx.sink.record(ResilienceEvent::RateLimitExceeded);
                        return Err(CallError::RateLimited { retry_after });
                    },
                    Err(error) => return Err(map_acquire_error(error)),
                }
                run_operation_with_shells(ctx, idx + 1, f).await
            },
            Step::LoadShed(predicate) => {
                if predicate() {
                    ctx.sink.record(ResilienceEvent::LoadShed);
                    Err(CallError::LoadShed)
                } else {
                    run_operation_with_shells(ctx, idx + 1, f).await
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
    cb: &CircuitBreaker,
    result: &Result<T, CallError<E>>,
    classifier: Option<&Arc<dyn ErrorClassifier<E>>>,
    duration: Option<Duration>,
) -> Outcome {
    match result {
        Ok(_) => duration.map_or(Outcome::Success, |duration| {
            cb.classify_outcome(true, duration)
        }),
        Err(CallError::Operation(e)) => classifier.map_or_else(
            || {
                duration.map_or(Outcome::Failure, |duration| {
                    cb.classify_outcome(false, duration)
                })
            },
            |c| classify_error_cb_outcome(cb, c.classify(e), duration),
        ),
        Err(CallError::RetriesExhausted { last, .. }) => classifier.map_or_else(
            || {
                duration.map_or(Outcome::Failure, |duration| {
                    cb.classify_outcome(false, duration)
                })
            },
            |c| classify_error_cb_outcome(cb, c.classify(last), duration),
        ),
        Err(CallError::Timeout(_)) => Outcome::Timeout,
        Err(_) => Outcome::Cancelled,
    }
}

fn classify_error_cb_outcome(
    cb: &CircuitBreaker,
    class: ErrorClass,
    duration: Option<Duration>,
) -> Outcome {
    duration.map_or_else(
        || class.into(),
        |duration| cb.classify_error_outcome(class, duration),
    )
}

/// Execute the Retry step of the pipeline.
#[allow(clippy::excessive_nesting)]
async fn run_retry_step<T, E, F>(
    config: &RetryConfig<E>,
    ctx: PipelineRunContext<E>,
    idx: usize,
    f: Arc<F>,
) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    let config_classifier = config.classifier.clone();
    let has_explicit_retry_classifier = config_classifier.is_some();
    let pipeline_classifier = ctx.classifier.clone();
    let mut inner_config =
        RetryConfig::<RetryStepError<E>>::from_nonzero_attempts(config.max_attempts())
            .backoff(config.backoff_config().clone())
            .jitter(config.jitter_config().clone());
    if let Some(total_budget) = config.total_budget_config() {
        inner_config = inner_config.total_budget(total_budget);
    }
    inner_config.sink = if ctx.sink_overrides_steps {
        Arc::clone(&ctx.sink)
    } else {
        Arc::clone(&config.sink)
    };
    inner_config.classifier = Some(Arc::new(FnClassifier::new(
        move |e: &RetryStepError<E>| match e {
            RetryStepError::Operation { error, .. } => config_classifier.as_ref().map_or_else(
                || {
                    pipeline_classifier
                        .as_ref()
                        .map_or(ErrorClass::Permanent, |classifier| {
                            classifier.classify(error)
                        })
                },
                |classifier| classifier.classify(error),
            ),
            RetryStepError::RetryablePattern(_) if has_explicit_retry_classifier => {
                ErrorClass::Permanent
            },
            RetryStepError::RetryablePattern(_) => ErrorClass::Transient,
            RetryStepError::FatalPattern(_) => ErrorClass::Permanent,
        },
    )));
    inner_config.on_retry = config.on_retry.as_ref().map(|notify| {
        let notify = Arc::clone(notify);
        Arc::new(
            move |e: &RetryStepError<E>, delay: Duration, attempt: u32| {
                if let RetryStepError::Operation { error, .. } = e {
                    notify(error, delay, attempt);
                }
            },
        ) as Arc<dyn Fn(&RetryStepError<E>, Duration, u32) + Send + Sync>
    });

    let retry_future = retry_with(inner_config, {
        let ctx = ctx.clone();
        let f = Arc::clone(&f);
        move || {
            let ctx = ctx.clone();
            let f = Arc::clone(&f);
            Box::pin(async move {
                let retry_hint = ctx.retry_hint.clone();
                classify_inner(
                    run_operation_with_shells(ctx, idx + 1, f).await,
                    retry_hint.as_ref(),
                )
            })
        }
    });

    let result = if let Some(cancellation) = ctx.cancellation.clone() {
        tokio::select! {
            result = retry_future => result,
            () = cancellation.token().cancelled() => Err(cancellation.cancelled_error()),
        }
    } else {
        retry_future.await
    };

    map_retry_result(result)
}

enum RetryStepError<E> {
    Operation {
        error: E,
        retry_after: Option<Duration>,
    },
    RetryablePattern(CallError<E>),
    FatalPattern(CallError<E>),
}

impl<E> RetryStepError<E> {
    fn into_call_error(self, exhausted_attempts: Option<u32>) -> CallError<E> {
        match self {
            Self::Operation { error, .. } => match exhausted_attempts {
                Some(attempts) => CallError::RetriesExhausted {
                    attempts,
                    last: error,
                },
                None => CallError::Operation(error),
            },
            Self::RetryablePattern(err) | Self::FatalPattern(err) => err,
        }
    }
}

impl<E> nebula_error::Classify for RetryStepError<E> {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Operation { .. } => nebula_error::ErrorCategory::External,
            Self::RetryablePattern(CallError::Timeout(_)) => nebula_error::ErrorCategory::Timeout,
            Self::RetryablePattern(CallError::RateLimited { .. }) => {
                nebula_error::ErrorCategory::RateLimit
            },
            Self::RetryablePattern(CallError::BulkheadFull) => {
                nebula_error::ErrorCategory::Exhausted
            },
            Self::RetryablePattern(_) | Self::FatalPattern(_) => {
                nebula_error::ErrorCategory::Internal
            },
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::Operation { .. } => {
                nebula_error::ErrorCode::new("RESILIENCE:PIPELINE_RETRY_OPERATION")
            },
            Self::RetryablePattern(CallError::Timeout(_)) => {
                nebula_error::ErrorCode::new("RESILIENCE:TIMEOUT")
            },
            Self::RetryablePattern(CallError::RateLimited { .. }) => {
                nebula_error::ErrorCode::new("RESILIENCE:RATE_LIMITED")
            },
            Self::RetryablePattern(CallError::BulkheadFull) => {
                nebula_error::ErrorCode::new("RESILIENCE:BULKHEAD_FULL")
            },
            Self::RetryablePattern(_) | Self::FatalPattern(_) => {
                nebula_error::ErrorCode::new("RESILIENCE:PIPELINE_RETRY_PATTERN")
            },
        }
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::Operation {
                retry_after: Some(after),
                ..
            }
            | Self::RetryablePattern(CallError::RateLimited {
                retry_after: Some(after),
            }) => Some(nebula_error::RetryHint::after(*after)),
            _ => None,
        }
    }
}

/// Classify an inner pipeline result for the retry layer.
///
/// `Operation` errors use the retry classifier for `E`; retryable pattern errors
/// (`Timeout`, `RateLimited`, `BulkheadFull`) can be retried by layer order; all
/// other pattern errors stop the retry loop immediately.
fn classify_inner<T, E>(
    result: Result<T, CallError<E>>,
    retry_hint: Option<&RetryHintFn<E>>,
) -> Result<T, RetryStepError<E>> {
    match result {
        Ok(v) => Ok(v),
        Err(CallError::Operation(error)) => {
            let retry_after = retry_hint.and_then(|hint| hint(&error));
            Err(RetryStepError::Operation { error, retry_after })
        },
        Err(other) if other.is_retryable() => Err(RetryStepError::RetryablePattern(other)),
        Err(other) => Err(RetryStepError::FatalPattern(other)),
    }
}

/// Map retry-step errors back to the public `CallError<E>` shape after retry.
fn map_retry_result<T, E>(
    result: Result<T, CallError<RetryStepError<E>>>,
) -> Result<T, CallError<E>> {
    match result {
        Ok(v) => Ok(v),
        Err(e) => Err(e.flat_map_inner(
            |inner| inner.into_call_error(None),
            |attempts, inner| inner.into_call_error(Some(attempts)),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt,
        future::ready,
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use nebula_error::{Classify, ErrorCategory, ErrorCode, RetryHint, codes};

    use super::*;
    use crate::{
        CallError, CancellationContext, CircuitBreaker, PolicyContext, RecordingSink,
        ResilienceEventKind, retry::BackoffConfig,
    };

    #[derive(Debug, Clone, Copy)]
    struct RetryAfterErr;

    impl fmt::Display for RetryAfterErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("retry after")
        }
    }

    impl Classify for RetryAfterErr {
        fn category(&self) -> ErrorCategory {
            ErrorCategory::RateLimit
        }

        fn code(&self) -> ErrorCode {
            codes::INTERNAL
        }

        fn retry_hint(&self) -> Option<RetryHint> {
            Some(RetryHint::after(Duration::from_millis(25)))
        }
    }

    async fn delay_first_two_attempts(attempt: u32) {
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn fail_transient_after_count(
        seen: Arc<AtomicU32>,
    ) -> Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>> {
        Box::pin(async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Err("transient")
        })
    }

    fn reject_first_rate_limit_check(
        checks: &AtomicU32,
        retry_after: Duration,
    ) -> Result<(), CallError<()>> {
        if checks.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(CallError::rate_limited_after(retry_after))
        } else {
            Ok(())
        }
    }

    fn always_reject_rate_limit_check(checks: &AtomicU32) -> Result<(), CallError<()>> {
        checks.fetch_add(1, Ordering::SeqCst);
        Err(CallError::rate_limited())
    }

    fn fail_once_with_retry_hint(attempts: &AtomicU32) -> Result<u32, RetryAfterErr> {
        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(RetryAfterErr);
        }
        Ok(42)
    }

    fn boxed_ok_static_operation(
        value: u32,
    ) -> Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>> {
        Box::pin(ready(Ok(value)))
    }

    async fn long_static_operation() -> Result<u32, &'static str> {
        tokio::time::sleep(Duration::from_mins(1)).await;
        Ok(42)
    }

    fn boxed_long_static_operation()
    -> Pin<Box<dyn Future<Output = Result<u32, &'static str>> + Send>> {
        Box::pin(long_static_operation())
    }

    async fn long_fallback_after_notify(
        started: Arc<tokio::sync::Notify>,
    ) -> Result<u32, CallError<()>> {
        started.notify_one();
        tokio::time::sleep(Duration::from_mins(1)).await;
        Ok(99)
    }

    #[tokio::test]
    async fn pipeline_timeout_wraps_retry() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();

        let pipeline = ResiliencePipeline::<&str>::builder()
            .timeout(Duration::from_secs(5))
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::from_millis(1)))
                    .retry_if(|_: &&str| true),
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

    #[test]
    fn build_checked_rejects_out_of_order_steps() {
        let err = ResiliencePipeline::<&str>::builder()
            .retry(RetryConfig::new(2).unwrap())
            .rate_limiter(Arc::new(|| Box::pin(async { Ok(()) })))
            .build_checked()
            .unwrap_err();

        assert_eq!(err.field, "pipeline_order");
    }

    #[test]
    fn build_checked_accepts_recommended_order() {
        let result = ResiliencePipeline::<&str>::builder()
            .load_shed(Arc::new(|| false))
            .rate_limiter(Arc::new(|| Box::pin(async { Ok(()) })))
            .timeout(Duration::from_secs(1))
            .retry(RetryConfig::new(2).unwrap())
            .build_checked();

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn pipeline_retry_retries_inner_timeout() {
        let attempts = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&attempts);

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .timeout(Duration::from_millis(10))
            .build();

        let result = pipeline
            .call(move || {
                let seen = Arc::clone(&seen);
                Box::pin(async move {
                    let attempt = seen.fetch_add(1, Ordering::SeqCst);
                    delay_first_two_attempts(attempt).await;
                    Ok::<u32, &str>(42)
                })
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn pipeline_retry_retries_inner_rate_limit_and_respects_retry_after() {
        let checks = Arc::new(AtomicU32::new(0));
        let seen_checks = Arc::clone(&checks);
        let operations = Arc::new(AtomicU32::new(0));
        let seen_operations = Arc::clone(&operations);
        let retry_after = Duration::from_millis(25);

        let rate_limiter: RateLimitCheck = Arc::new(move || {
            let seen_checks = Arc::clone(&seen_checks);
            Box::pin(async move { reject_first_rate_limit_check(&seen_checks, retry_after) })
        });

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(2)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .rate_limiter(rate_limiter)
            .build();

        let start = std::time::Instant::now();
        let result = pipeline
            .call(move || {
                let seen_operations = Arc::clone(&seen_operations);
                Box::pin(async move {
                    seen_operations.fetch_add(1, Ordering::SeqCst);
                    Ok::<u32, &str>(42)
                })
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(checks.load(Ordering::SeqCst), 2);
        assert_eq!(operations.load(Ordering::SeqCst), 1);
        assert!(
            start.elapsed() >= Duration::from_millis(20),
            "retry_after should act as a retry delay floor"
        );
    }

    #[tokio::test]
    async fn pipeline_retry_respects_operation_retry_hint_from_classify_errors() {
        let attempts = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&attempts);

        let pipeline = ResiliencePipeline::<RetryAfterErr>::builder()
            .classify_errors()
            .retry(
                RetryConfig::new(2)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .build();

        let start = std::time::Instant::now();
        let result = pipeline
            .call(move || {
                let seen = Arc::clone(&seen);
                Box::pin(async move { fail_once_with_retry_hint(&seen) })
            })
            .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert!(
            start.elapsed() >= Duration::from_millis(20),
            "Classify retry_hint should act as a retry delay floor"
        );
    }

    #[test]
    fn classify_errors_preserves_user_retry_hint() {
        let builder = ResiliencePipeline::<RetryAfterErr>::builder()
            .retry_hint(|_: &RetryAfterErr| Some(Duration::from_millis(5)))
            .classify_errors();

        let hint = builder
            .retry_hint
            .as_ref()
            .and_then(|hint| hint(&RetryAfterErr));

        assert_eq!(hint, Some(Duration::from_millis(5)));
    }

    #[tokio::test]
    async fn pipeline_retry_does_not_retry_inner_circuit_open() {
        let cb = Arc::new(CircuitBreaker::new(crate::CircuitBreakerConfig::default()).unwrap());
        cb.force_open();
        let operations = Arc::new(AtomicU32::new(0));
        let seen_operations = Arc::clone(&operations);

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .circuit_breaker(cb)
            .build();

        let result = pipeline
            .call(move || {
                let seen_operations = Arc::clone(&seen_operations);
                Box::pin(async move {
                    seen_operations.fetch_add(1, Ordering::SeqCst);
                    Ok::<u32, &str>(42)
                })
            })
            .await;

        assert!(matches!(result, Err(CallError::CircuitOpen)));
        assert_eq!(operations.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn build_recommended_order_rejects_before_retry() {
        let checks = Arc::new(AtomicU32::new(0));
        let seen_checks = Arc::clone(&checks);
        let operations = Arc::new(AtomicU32::new(0));
        let seen_operations = Arc::clone(&operations);

        let rate_limiter: RateLimitCheck = Arc::new(move || {
            let seen_checks = Arc::clone(&seen_checks);
            Box::pin(async move { always_reject_rate_limit_check(&seen_checks) })
        });

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .rate_limiter(rate_limiter)
            .build_recommended_order();

        let result = pipeline
            .call(move || {
                let seen_operations = Arc::clone(&seen_operations);
                Box::pin(async move {
                    seen_operations.fetch_add(1, Ordering::SeqCst);
                    Ok::<u32, &str>(42)
                })
            })
            .await;

        assert!(matches!(result, Err(CallError::RateLimited { .. })));
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert_eq!(operations.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn pipeline_with_sink_overrides_retry_config_sink() {
        let sink = RecordingSink::new();

        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .retry(
                RetryConfig::new(2)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO))
                    .retry_if(|_: &&str| true),
            )
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
            .await;

        assert!(matches!(
            result,
            Err(CallError::RetriesExhausted { attempts: 2, .. })
        ));
        assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 2);
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
    async fn pipeline_retry_does_not_replay_unknown_operation_errors_by_default() {
        let attempts = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&attempts);

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::ZERO)),
            )
            .build();

        let result = pipeline
            .call(move || {
                let seen = Arc::clone(&seen);
                Box::pin(async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Err::<u32, &str>("unknown")
                })
            })
            .await;

        assert!(matches!(result, Err(CallError::Operation("unknown"))));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pipeline_context_cancels_retry_sleep() {
        let attempts = Arc::new(AtomicU32::new(0));
        let seen = Arc::clone(&attempts);
        let cancellation = CancellationContext::with_reason("shutdown");
        let cancellation_for_call = cancellation.clone();

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(
                RetryConfig::new(3)
                    .unwrap()
                    .backoff(BackoffConfig::Fixed(Duration::from_secs(10)))
                    .retry_if(|_: &&str| true),
            )
            .build();

        let task = tokio::spawn(async move {
            pipeline
                .call_with_context(&cancellation_for_call, move || {
                    fail_transient_after_count(Arc::clone(&seen))
                })
                .await
        });

        while attempts.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        cancellation.cancel();

        let result = tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .expect("pipeline should stop during retry sleep")
            .expect("task should not panic");

        assert!(matches!(
            result,
            Err(CallError::Cancelled { reason: Some(reason) }) if reason == "shutdown"
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
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
    async fn pipeline_retry_classifier_blocks_inner_pattern_retries() {
        let checks = Arc::new(AtomicU32::new(0));
        let seen_checks = Arc::clone(&checks);
        let rate_limiter: RateLimitCheck = Arc::new(move || {
            let seen_checks = Arc::clone(&seen_checks);
            Box::pin(async move {
                reject_first_rate_limit_check(&seen_checks, Duration::from_millis(50))
            })
        });

        let pipeline = ResiliencePipeline::<&str>::builder()
            .retry(RetryConfig::new(3).unwrap().retry_if(|_: &&str| false))
            .rate_limiter(rate_limiter)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        assert!(matches!(result, Err(CallError::RateLimited { .. })));
        assert_eq!(checks.load(Ordering::SeqCst), 1);
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
                    .retry_if(|_: &&str| true)
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
    async fn pipeline_with_sink_does_not_double_count_prebuilt_bulkhead_rejection() {
        let sink = RecordingSink::new();
        let bh = Arc::new(
            Bulkhead::new(crate::BulkheadConfig {
                max_concurrency: 1,
                queue_size: 0,
                timeout: None,
            })
            .unwrap()
            .with_sink(sink.clone()),
        );
        let _permit = bh.acquire::<&str>().await.unwrap();

        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .bulkhead(bh)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        assert!(matches!(result, Err(CallError::BulkheadFull)));
        assert_eq!(sink.count(ResilienceEventKind::BulkheadRejected), 1);
    }

    #[tokio::test]
    async fn pipeline_with_sink_does_not_double_count_prebuilt_circuit_state_change() {
        use crate::sink::CircuitState;

        let sink = RecordingSink::new();
        let cb = Arc::new(
            CircuitBreaker::new(crate::CircuitBreakerConfig {
                failure_threshold: 1,
                min_operations: 1,
                ..Default::default()
            })
            .unwrap()
            .with_sink(sink.clone()),
        );

        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .circuit_breaker(cb)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Err::<u32, &str>("fail") }))
            .await;

        assert!(matches!(result, Err(CallError::Operation("fail"))));
        assert!(sink.has_state_change(CircuitState::Open));
        assert_eq!(sink.count(ResilienceEventKind::CircuitStateChanged), 1);
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

        let sink = RecordingSink::new();
        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .timeout(Duration::from_millis(10))
            .build();

        let fallback = ValueFallback::new(99u32);

        let result = pipeline
            .call_with_fallback(
                || {
                    Box::pin(async {
                        std::future::pending::<()>().await;
                        Ok::<u32, &str>(42)
                    })
                },
                &fallback,
            )
            .await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
        assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 1);
        assert_eq!(sink.count(ResilienceEventKind::PipelineCompleted), 1);

        let events = sink.events();
        let completed = events.iter().find_map(|event| {
            if let ResilienceEvent::PipelineCompleted { scope, outcome } = event {
                Some((scope, outcome))
            } else {
                None
            }
        });
        assert!(matches!(
            completed,
            Some((
                _,
                PipelineOutcome::FallbackSucceeded {
                    primary_error: crate::CallErrorKind::Timeout,
                }
            ))
        ));
    }

    #[tokio::test]
    async fn pipeline_context_fallback_does_not_recover_cancellation() {
        struct AlwaysFallback {
            calls: Arc<AtomicU32>,
        }

        impl crate::fallback::FallbackStrategy<u32, &'static str> for AlwaysFallback {
            fn recover<'a>(
                &'a self,
                _error: CallError<&'static str>,
            ) -> Pin<Box<dyn Future<Output = Result<u32, CallError<&'static str>>> + Send + 'a>>
            {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(ready(Ok(99)))
            }

            fn should_fallback(&self, _error: &CallError<&'static str>) -> bool {
                true
            }
        }

        let calls = Arc::new(AtomicU32::new(0));
        let fallback = AlwaysFallback {
            calls: Arc::clone(&calls),
        };
        let cancellation = CancellationContext::with_reason("shutdown");
        cancellation.cancel();
        let pipeline = ResiliencePipeline::<&'static str>::builder().build();

        let result = pipeline
            .call_with_context_and_fallback(
                &cancellation,
                || boxed_ok_static_operation(42),
                &fallback,
            )
            .await;

        assert!(matches!(result, Err(CallError::Cancelled { .. })));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn pipeline_context_cancels_inflight_fallback() {
        use crate::fallback::FunctionFallback;

        let cancellation = CancellationContext::with_reason("shutdown");
        let cancellation_for_call = cancellation.clone();
        let started = Arc::new(tokio::sync::Notify::new());
        let started_for_fallback = Arc::clone(&started);

        let task = tokio::spawn(async move {
            let pipeline = ResiliencePipeline::<&'static str>::builder()
                .timeout(Duration::from_millis(1))
                .build();
            let fallback = FunctionFallback::new(move |_err: CallError<()>| {
                let started = Arc::clone(&started_for_fallback);
                long_fallback_after_notify(started)
            });

            pipeline
                .call_with_context_and_fallback(
                    &cancellation_for_call,
                    boxed_long_static_operation,
                    &fallback,
                )
                .await
        });

        started.notified().await;
        cancellation.cancel();

        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();

        assert!(matches!(result, Err(CallError::Cancelled { .. })));
    }

    #[tokio::test]
    async fn policy_context_deadline_bounds_entire_pipeline() {
        let sink = RecordingSink::new();
        let context = PolicyContext::with_timeout(Duration::from_millis(1))
            .with_scope(PolicyScope::empty().tenant_id("tenant-context"));
        let pipeline = ResiliencePipeline::<&'static str>::builder()
            .with_sink(sink.clone())
            .scope(PolicyScope::empty().tenant_id("tenant-builder"))
            .build();

        let result = pipeline
            .call_with_policy_context(&context, boxed_long_static_operation)
            .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
        assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 1);

        let events = sink.events();
        let completed = events.iter().find_map(|event| {
            if let ResilienceEvent::PipelineCompleted { scope, outcome } = event {
                Some((scope, outcome))
            } else {
                None
            }
        });
        assert!(matches!(
            completed,
            Some((
                scope,
                PipelineOutcome::Failure {
                    error: crate::CallErrorKind::Timeout,
                }
            )) if scope.tenant_id.as_deref() == Some("tenant-context")
        ));
    }

    #[tokio::test]
    async fn policy_context_deadline_bounds_inflight_fallback() {
        use crate::fallback::FunctionFallback;

        let sink = RecordingSink::new();
        let started = Arc::new(tokio::sync::Notify::new());
        let started_for_fallback = Arc::clone(&started);
        let context = PolicyContext::with_timeout(Duration::from_millis(50));
        let pipeline = ResiliencePipeline::<&'static str>::builder()
            .with_sink(sink.clone())
            .timeout(Duration::from_millis(1))
            .build();
        let fallback = FunctionFallback::new(move |_err: CallError<()>| {
            let started = Arc::clone(&started_for_fallback);
            long_fallback_after_notify(started)
        });

        let call = pipeline.call_with_policy_context_and_fallback(
            &context,
            boxed_long_static_operation,
            &fallback,
        );
        tokio::pin!(call);
        tokio::select! {
            () = started.notified() => {},
            result = &mut call => panic!("fallback did not start before call completed: {result:?}"),
        }

        let result = call.await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
        assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
        assert_eq!(sink.count(ResilienceEventKind::TimeoutElapsed), 2);
        assert_eq!(sink.count(ResilienceEventKind::PipelineCompleted), 1);
    }

    #[tokio::test]
    async fn pipeline_completion_event_carries_scope() {
        let sink = RecordingSink::new();
        let pipeline = ResiliencePipeline::<&str>::builder()
            .with_sink(sink.clone())
            .scope(
                PolicyScope::empty()
                    .tenant_id("tenant-a")
                    .operation("gmail.poll"),
            )
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        assert_eq!(result.unwrap(), 42);
        let events = sink.events();
        let completed = events.iter().find_map(|event| {
            if let ResilienceEvent::PipelineCompleted { scope, outcome } = event {
                Some((scope, outcome))
            } else {
                None
            }
        });

        assert!(matches!(
            completed,
            Some((scope, PipelineOutcome::Success))
                if scope.tenant_id.as_deref() == Some("tenant-a")
                    && scope.operation.as_deref() == Some("gmail.poll")
        ));
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
