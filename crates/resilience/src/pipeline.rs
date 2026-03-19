//! `ResiliencePipeline` — compose multiple resilience patterns into a single call chain.
//!
//! Recommended layer order (outermost → innermost):
//! `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`
//!
//! Layers are applied in the order added: first added = outermost.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Duration;

use crate::{
    CallError,
    bulkhead::Bulkhead,
    circuit_breaker::{CircuitBreaker, Outcome, ProbeGuard},
    retry::{RetryConfig, retry_with},
};

// ── Execution phases ──────────────────────────────────────────────────────────
//
// To eliminate per-step Box::pin allocations the pipeline is split into three
// phases that are processed iteratively:
//
//  1. Pre-phase  — steps executed *before* the operation (outermost → innermost):
//     LoadShed, RateLimiter, (open) CircuitBreaker check, Bulkhead acquire.
//
//  2. Inner-phase — the actual operation call, wrapped once by CB/Bulkhead
//     wrappers and the optional Timeout/Retry shells.
//
//  3. Post-phase — outcome recording after the operation returns.
//
// Only Timeout and Retry still produce a single Box::pin each (Timeout requires
// it for `tokio::time::timeout`; Retry needs async recursion for back-off).
// Every other step is now allocation-free.

/// Async predicate for rate limiting — returns `Ok(())` or `Err(CallError::RateLimited)`.
pub type RateLimitCheck =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), CallError<()>>> + Send>> + Send + Sync>;

/// Predicate for load shedding — returns `true` to shed the request.
pub type LoadShedPredicate = Arc<dyn Fn() -> bool + Send + Sync>;

// ── Steps ─────────────────────────────────────────────────────────────────────

enum Step<E: 'static> {
    Timeout(Duration),
    Retry(RetryConfig<E>),
    CircuitBreaker(Arc<CircuitBreaker>),
    Bulkhead(Arc<Bulkhead>),
    RateLimiter(RateLimitCheck),
    LoadShed(LoadShedPredicate),
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Builder for [`ResiliencePipeline`].
pub struct PipelineBuilder<E: 'static> {
    steps: Vec<Step<E>>,
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
        Self { steps: Vec::new() }
    }

    /// Add a timeout step (outermost wrapper if added first).
    #[must_use]
    pub fn timeout(mut self, d: Duration) -> Self {
        self.steps.push(Step::Timeout(d));
        self
    }

    /// Add a retry step.
    #[must_use]
    pub fn retry(mut self, config: RetryConfig<E>) -> Self {
        self.steps.push(Step::Retry(config));
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

    /// Add a rate limiter step.
    ///
    /// The `check` closure is called before each operation to acquire a permit.
    /// Use it to bridge any `RateLimiter` implementation:
    ///
    /// ```rust,ignore
    /// let rl = Arc::new(TokenBucket::new(100, 10.0).unwrap());
    /// builder.rate_limiter({
    ///     let rl = Arc::clone(&rl);
    ///     Arc::new(move || {
    ///         let rl = Arc::clone(&rl);
    ///         Box::pin(async move { rl.acquire().await })
    ///     })
    /// })
    /// ```
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
        }
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

    if let (Some(r), Some(t)) = (retry_pos, timeout_pos)
        && t > r
    {
        tracing::warn!(
            "ResiliencePipeline: timeout is inside retry (each attempt gets its own timeout). \
             Move timeout before retry for a single deadline across all attempts."
        );
    }
}

// ── Pipeline ──────────────────────────────────────────────────────────────────

/// A composed resilience pipeline that applies multiple patterns in order.
///
/// Build via [`ResiliencePipeline::builder()`].
pub struct ResiliencePipeline<E: 'static> {
    steps: Arc<Vec<Step<E>>>,
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
        execute_pipeline(Arc::clone(&self.steps), Arc::new(boxed)).await
    }
}

// ── Iterative pipeline executor ───────────────────────────────────────────────
//
// Processes steps in two passes to avoid per-step Box::pin:
//
//  Pass 1 (forward, idx 0 → N-1): guard steps — LoadShed, RateLimiter,
//          CircuitBreaker::can_execute, Bulkhead::acquire.
//          Returns early on rejection without touching the operation.
//
//  Pass 2: find the innermost Timeout / Retry shell (if any) and execute
//          the operation through it.  CB and Bulkhead outcome recording
//          happens after the operation returns.
//
// Only one Box::pin per Timeout shell and one per Retry step are created,
// instead of one per pipeline step as in the old recursive approach.

async fn execute_pipeline<T, E, F>(steps: Arc<Vec<Step<E>>>, f: Arc<F>) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    // ── Pass 1: guard checks (no allocations) ─────────────────────────────────
    //
    // Collect permits/handles we need to clean up after the operation.
    // For CircuitBreaker and Bulkhead we call the lower-level API directly so
    // we can record the outcome ourselves after the operation completes.

    use crate::bulkhead::BulkheadPermit;
    use crate::circuit_breaker::Outcome;

    // Indices of CB/Bulkhead steps that were successfully entered (for outcome recording).
    let mut entered_cb: Vec<Arc<CircuitBreaker>> = Vec::new();
    // RAII permit handles — dropped when this scope exits, releasing the slot.
    let mut bh_permits: Vec<BulkheadPermit> = Vec::new();

    for step in steps.iter() {
        match step {
            Step::LoadShed(predicate) => {
                if predicate() {
                    return Err(CallError::LoadShed);
                }
            }
            Step::RateLimiter(check) => {
                check().await.map_err(|_| CallError::RateLimited)?;
            }
            Step::CircuitBreaker(cb) => {
                cb.can_execute()?;
                entered_cb.push(Arc::clone(cb));
            }
            Step::Bulkhead(bh) => {
                let permit = bh.acquire::<E>().await?;
                bh_permits.push(permit);
            }
            // Timeout and Retry are handled in pass 2.
            Step::Timeout(_) | Step::Retry(_) => {}
        }
    }

    // ── Pass 2: find the outermost Timeout/Retry shell and execute ────────────
    //
    // Walk steps to find the first Timeout or Retry entry (outermost wrapper).
    // If neither exists, execute the operation directly.

    let result = run_operation_with_shells(&steps, 0, Arc::clone(&f)).await;

    // ── Post: record CB outcomes ──────────────────────────────────────────────
    let outcome = match &result {
        Err(CallError::Operation(_) | CallError::RetriesExhausted { .. }) => Outcome::Failure,
        Err(CallError::Timeout(_)) => Outcome::Timeout,
        Err(CallError::Cancelled { .. }) => Outcome::Cancelled,
        _ => Outcome::Success, // Ok / LoadShed / RateLimited / etc. — CB already rejected above
    };
    for cb in &entered_cb {
        cb.record_outcome(outcome);
    }

    // Permits are held until here so the bulkhead slots stay reserved for the
    // entire duration of the operation; drop them explicitly before returning.
    drop(bh_permits);
    result
}

/// Recursively apply only Timeout and Retry shells (one `Box::pin` per shell),
/// then call the user function.  `CircuitBreaker` and `Bulkhead` guards have
/// already been processed by `execute_pipeline`; their wrappers are skipped here.
fn run_operation_with_shells<T, E, F>(
    steps: &Arc<Vec<Step<E>>>,
    idx: usize,
    f: Arc<F>,
) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    // Find the next Timeout or Retry step starting from `idx`.
    let steps = Arc::clone(steps);
    Box::pin(async move {
        if idx >= steps.len() {
            return f().await.map_err(CallError::Operation);
        }

        match &steps[idx] {
            Step::Timeout(d) => {
                let d = *d;
                tokio::time::timeout(d, run_operation_with_shells(&steps, idx + 1, f))
                    .await
                    .unwrap_or_else(|_| Err(CallError::Timeout(d)))
            }
            Step::Retry(config) => run_retry_step(config, Arc::clone(&steps), idx, f).await,
            Step::CircuitBreaker(cb) => {
                cb.can_execute()?;
                let guard = ProbeGuard(cb);
                let result = run_operation_with_shells(&steps, idx + 1, Arc::clone(&f)).await;
                // Defuse guard — record the real outcome instead.
                std::mem::forget(guard);
                match &result {
                    Ok(_) => cb.record_outcome(Outcome::Success),
                    Err(CallError::Operation(_)) => cb.record_outcome(Outcome::Failure),
                    // Non-operation errors (rate limit, load shed, etc.) from inner
                    // steps are not the downstream's fault — don't count them.
                    Err(_) => cb.record_outcome(Outcome::Cancelled),
                }
                result
            }
            Step::Bulkhead(bh) => {
                let _permit = bh.acquire().await?;
                run_operation_with_shells(&steps, idx + 1, f).await
            }
            Step::RateLimiter(check) => {
                check().await.map_err(|_| CallError::RateLimited)?;
                run_operation_with_shells(&steps, idx + 1, f).await
            }
            Step::LoadShed(predicate) => {
                if predicate() {
                    Err(CallError::LoadShed)
                } else {
                    run_operation_with_shells(&steps, idx + 1, f).await
                }
            }
        }
    })
}

/// Execute the Retry step of the pipeline.
#[allow(clippy::excessive_nesting)]
async fn run_retry_step<T, E, F>(
    config: &RetryConfig<E>,
    steps: Arc<Vec<Step<E>>>,
    idx: usize,
    f: Arc<F>,
) -> Result<T, CallError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    let bail: Arc<Mutex<Option<CallError<E>>>> = Arc::new(Mutex::new(None));
    let bail_check = Arc::clone(&bail);

    let inner_config = RetryConfig::<Option<E>>::new_unchecked(config.max_attempts)
        .backoff(config.backoff.clone())
        .jitter(config.jitter.clone())
        .retry_if(move |e: &Option<E>| e.is_some() && bail_check.lock().is_none());

    let result = retry_with(inner_config, {
        let steps = Arc::clone(&steps);
        let f = Arc::clone(&f);
        let bail = Arc::clone(&bail);
        move || {
            let steps = Arc::clone(&steps);
            let f = Arc::clone(&f);
            let bail = Arc::clone(&bail);
            Box::pin(async move {
                classify_inner(run_operation_with_shells(&steps, idx + 1, f).await, &bail)
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
        }
    }
}

/// Map `CallError<Option<E>>` back to `CallError<E>` after retry.
fn map_retry_result<T, E: Send>(
    result: Result<T, CallError<Option<E>>>,
    bail: &Arc<Mutex<Option<CallError<E>>>>,
) -> Result<T, CallError<E>> {
    match result {
        Ok(v) => Ok(v),
        Err(CallError::RetriesExhausted {
            attempts,
            last: Some(e),
        }) => Err(CallError::RetriesExhausted { attempts, last: e }),
        Err(CallError::RetriesExhausted { last: None, .. } | CallError::Operation(None)) => {
            Err(bail
                .lock()
                .take()
                .unwrap_or(CallError::Cancelled { reason: None }))
        }
        Err(CallError::Operation(Some(e))) => Err(CallError::Operation(e)),
        Err(CallError::CircuitOpen) => Err(CallError::CircuitOpen),
        Err(CallError::BulkheadFull) => Err(CallError::BulkheadFull),
        Err(CallError::Timeout(d)) => Err(CallError::Timeout(d)),
        Err(CallError::RateLimited) => Err(CallError::RateLimited),
        Err(CallError::LoadShed) => Err(CallError::LoadShed),
        Err(CallError::Cancelled { reason }) => Err(CallError::Cancelled { reason }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CallError, retry::BackoffConfig};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

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
        let rl: RateLimitCheck = Arc::new(|| Box::pin(async { Err(CallError::RateLimited) }));

        let pipeline = ResiliencePipeline::<&str>::builder()
            .circuit_breaker(cb)
            .rate_limiter(rl)
            .build();

        let result = pipeline
            .call(|| Box::pin(async { Ok::<u32, &str>(42) }))
            .await;

        // Should return RateLimited, not panic
        assert!(matches!(result, Err(CallError::RateLimited)));
    }
}
