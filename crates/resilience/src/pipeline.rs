//! `ResiliencePipeline` — compose multiple resilience patterns into a single call chain.
//!
//! Recommended layer order (outermost → innermost):
//! `timeout → retry → circuit_breaker → bulkhead`
//!
//! Layers are applied in the order added: first added = outermost.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::Duration;

use crate::{
    CallError,
    patterns::{
        bulkhead::Bulkhead,
        circuit_breaker::CircuitBreaker,
        retry::{RetryConfig, retry_with},
    },
};

// ── Steps ─────────────────────────────────────────────────────────────────────

enum Step<E: 'static> {
    Timeout(Duration),
    Retry(RetryConfig<E>),
    CircuitBreaker(Arc<CircuitBreaker>),
    Bulkhead(Arc<Bulkhead>),
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
    pub async fn call<T, F>(&self, f: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        run_steps(Arc::clone(&self.steps), 0, Arc::new(f)).await
    }
}

// ── Recursive step executor ───────────────────────────────────────────────────

fn run_steps<T, E, F>(
    steps: Arc<Vec<Step<E>>>,
    idx: usize,
    f: Arc<F>,
) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    Box::pin(async move {
        if idx >= steps.len() {
            return f().await.map_err(CallError::Operation);
        }

        match &steps[idx] {
            Step::Timeout(d) => {
                let d = *d;
                tokio::time::timeout(d, run_steps(steps, idx + 1, f))
                    .await
                    .unwrap_or_else(|_| Err(CallError::Timeout(d)))
            }
            Step::Retry(config) => run_retry_step(config, Arc::clone(&steps), idx, f).await,
            Step::CircuitBreaker(cb) => {
                let cb = Arc::clone(cb);
                cb.call(move || run_inner_unwrapped(steps, idx, f, "CircuitBreaker"))
                    .await
            }
            Step::Bulkhead(bh) => {
                let bh = Arc::clone(bh);
                bh.call(move || run_inner_unwrapped(steps, idx, f, "Bulkhead"))
                    .await
            }
        }
    })
}

/// Unwrap inner pipeline steps for use inside CB / Bulkhead wrappers.
///
/// Non-`Operation` errors are unreachable in a well-ordered pipeline
/// (timeout before circuit breaker / bulkhead).
fn run_inner_unwrapped<T, E, F>(
    steps: Arc<Vec<Step<E>>>,
    idx: usize,
    f: Arc<F>,
    step_name: &'static str,
) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>> + Send + Sync + 'static,
{
    Box::pin(async move {
        match run_steps(steps, idx + 1, f).await {
            Ok(v) => Ok(v),
            Err(CallError::Operation(e)) => Err(e),
            Err(_) => unreachable!(
                "ResiliencePipeline: non-Operation error inside {step_name} step; \
                 ensure timeout/retry are ordered before {step_name}"
            ),
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
            Box::pin(async move { classify_inner(run_steps(steps, idx + 1, f).await, &bail) })
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
    use crate::{CallError, patterns::retry::BackoffConfig};
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
}
