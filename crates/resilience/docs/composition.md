# nebula-resilience — Composition

`PipelineBuilder` and `ResiliencePipeline` let you compose individual resilience patterns
into an ordered middleware pipeline. Steps are applied in the order added: first added
= outermost (wraps all subsequent steps).

---

## Table of Contents

- [Recommended Step Order](#recommended-step-order)
- [PipelineBuilder API](#pipelinebuilder-api)
- [ResiliencePipeline Execution](#resiliencepipeline-execution)
- [Step Interactions](#step-interactions)
- [Layer Order Warning](#layer-order-warning)
- [Examples](#examples)

---

## Recommended Step Order

```
load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead
```

Why this order:
- `load_shed` and `rate_limiter` reject once before entering the expensive policy stack.
- `timeout` enforces a **single deadline across all retry attempts**.
- `retry` sits inside timeout so each attempt consumes from the same budget and retry
  sleeps cannot exceed the remaining budget.
- `circuit_breaker` is checked per attempt. When it opens, retry observes the policy
  rejection and stops unless a classifier explicitly marks that error retryable.
- `bulkhead` is innermost — concurrency is capped per individual attempt.

> **Note**: placing `timeout` *inside* `retry` gives each attempt its own independent
> deadline. `build()` emits a `tracing::warn!` if this ordering is detected.
> Placing `rate_limiter` inside `retry` can multiply rate-limit checks and retries;
> `build()` warns for that order too.

For config/schema-driven pipelines, prefer `build_checked()` so invalid order is a
configuration error instead of an operator-visible warning. Use
`build_recommended_order()` only when it is acceptable for the crate to sort different
policy kinds into the safe order.

---

## PipelineBuilder API

```rust
use nebula_resilience::{ResiliencePipeline, CallError};
use nebula_resilience::retry::{RetryConfig, BackoffConfig};
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::bulkhead::{Bulkhead, BulkheadConfig};
use std::sync::Arc;
use std::time::Duration;

let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig {
    failure_threshold: 5,
    reset_timeout: Duration::from_secs(30),
    ..Default::default()
})?);

let bh = Arc::new(Bulkhead::new(BulkheadConfig {
    max_concurrency: 20,
    ..Default::default()
})?);

let pipeline = ResiliencePipeline::<MyError>::builder()
    .timeout(Duration::from_secs(10))
    .retry(
        RetryConfig::new(3)?
            .backoff(BackoffConfig::exponential_default())
            .retry_if(|e: &MyError| e.is_transient()),
    )
    .circuit_breaker(cb)
    .bulkhead(bh)
    .build();
```

### Builder methods

```rust
impl<E: Send + 'static> PipelineBuilder<E> {
    pub const fn new() -> Self

    /// Add a hard deadline (outermost if first).
    #[must_use]
    pub fn timeout(self, d: Duration) -> Self

    /// Add a retry step with explicit config.
    #[must_use]
    pub fn retry(self, config: RetryConfig<E>) -> Self

    /// Inject a sink for pipeline-level timeout / rate-limit / load-shed events.
    #[must_use]
    pub fn with_sink(self, sink: impl MetricsSink + 'static) -> Self

    /// Attach workflow/resource scope to PipelineCompleted events.
    #[must_use]
    pub fn scope(self, scope: PolicyScope) -> Self

    /// Add a circuit breaker step. Takes Arc so it can be shared / inspected externally.
    #[must_use]
    pub fn circuit_breaker(self, cb: Arc<CircuitBreaker>) -> Self

    /// Add a bulkhead step. Takes Arc so it can be shared.
    #[must_use]
    pub fn bulkhead(self, bh: Arc<Bulkhead>) -> Self

    /// Add a rate limiter step from any `Arc<impl RateLimiter>`.
    #[must_use]
    pub fn rate_limiter_from<RL: RateLimiter + 'static>(self, rl: Arc<RL>) -> Self

    /// Add a rate limiter step from an object-safe registry entry.
    #[must_use]
    pub fn rate_limiter_erased(self, rl: Arc<dyn ErasedRateLimiter>) -> Self

    /// Add a custom rate limiter check.
    #[must_use]
    pub fn rate_limiter(self, check: RateLimitCheck) -> Self

    /// Add a load-shed predicate.
    #[must_use]
    pub fn load_shed(self, predicate: LoadShedPredicate) -> Self

    /// Sort steps into the recommended order before building.
    #[must_use]
    pub fn build_recommended_order(self) -> ResiliencePipeline<E>

    /// Build only if steps are already in the recommended order.
    pub fn build_checked(self) -> Result<ResiliencePipeline<E>, ConfigError>

    /// Consume the builder and return the pipeline.
    /// Emits tracing::warn! for surprising timeout/retry and rate-limit/retry order.
    #[must_use]
    pub fn build(self) -> ResiliencePipeline<E>
}
```

---

## ResiliencePipeline Execution

```rust
impl<E: Send + 'static> ResiliencePipeline<E> {
    pub const fn builder() -> PipelineBuilder<E>;

    pub async fn call<T, F>(&self, f: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>
            + Clone + Send + Sync + 'static;

    pub async fn call_with_context<T, F>(
        &self,
        cancellation: &CancellationContext,
        f: F,
    ) -> Result<T, CallError<E>>;

    pub async fn call_with_context_and_fallback<T, F>(
        &self,
        cancellation: &CancellationContext,
        f: F,
        fallback: &dyn FallbackStrategy<T, E>,
    ) -> Result<T, CallError<E>>;

    pub async fn call_with_policy_context<T, F>(
        &self,
        context: &PolicyContext,
        f: F,
    ) -> Result<T, CallError<E>>;

    pub async fn call_with_policy_context_and_fallback<T, F>(
        &self,
        context: &PolicyContext,
        f: F,
        fallback: &dyn FallbackStrategy<T, E>,
    ) -> Result<T, CallError<E>>;
}
```

`call()` accepts a factory closure `F` (not a single future) because some steps (retry,
circuit breaker) must be able to invoke the operation multiple times.
`call_with_context()` additionally makes timeout, retry sleep, bulkhead acquisition,
rate-limit checks, and the operation itself interruptible by a shared
`CancellationContext`.
`call_with_context_and_fallback()` carries the same cancellation contract into
fallback: cancellation wins before fallback starts and while the fallback future is
running, so engine shutdown is not converted into a successful fallback value.
`call_with_policy_context()` and `call_with_policy_context_and_fallback()` are the
workflow-runtime entry points when a call has one cancellation token, one total
deadline, and one telemetry scope. The context deadline bounds the whole call, not
only a single attempt or one pipeline step.

---

## Step Interactions

### Retry and error classification

Pipeline retry treats operation errors as permanent by default. To replay an
operation error, configure `RetryConfig::retry_if`, `RetryConfig::with_classifier`,
`PipelineBuilder::classifier`, or `PipelineBuilder::classify_errors()`.

Inner policy errors are classified by kind. Cancellation, load shedding, and an open
circuit are permanent by default. Timeout, rate limiting, and bulkhead rejection may
be retried when a classifier marks them transient; rate-limit and operation
`retry_hint().after` values are preserved as delay floors.

```
retry step receives:
  Ok(v)              → return Ok(v)
  Err(Operation(e))  → classifier/predicate says transient? sleep, try again : return Err(Operation(e))
  Err(CircuitOpen)   → permanent by default
  Err(LoadShed)      → permanent by default
  Err(Cancelled)     → permanent by default
  Err(RateLimited)   → retryable only when classified transient; retry_after is honored
  Err(BulkheadFull)  → retryable only when classified transient
  Err(Timeout)       → retryable only when classified transient
```

### `CircuitBreaker` and `Bulkhead` unwrapping

These steps operate on the shared `CallError<E>` model from the inner pipeline.
Structural errors such as `CircuitOpen`, `BulkheadFull`, `Timeout`, `RateLimited`,
and `LoadShed` propagate unchanged rather than being retried as operation failures.

---

## Layer Order Warning

`build()` inspects step positions and warns via `tracing::warn!` if timeout appears
after retry in the step list, or if rate limiting appears inside retry:

```rust
// This emits a warning at build time
let pipeline = ResiliencePipeline::<&str>::builder()
    .retry(RetryConfig::new(3)?.backoff(BackoffConfig::Fixed(Duration::from_millis(50))))
    .timeout(Duration::from_secs(5))  // ← inside retry: each attempt gets its own 5s
    .build();
```

```
WARN ResiliencePipeline: timeout is inside retry (each attempt gets its own timeout).
     Move timeout before retry for a single deadline across all attempts.
```

Use `build_recommended_order()` when policy declarations come from configuration and
you want the crate to sort different step kinds into:

```
load_shed -> rate_limiter -> timeout -> retry -> circuit_breaker -> bulkhead
```

Use `build_checked()` when the config source should be rejected instead:

```rust
let pipeline = ResiliencePipeline::<&str>::builder()
    .retry(RetryConfig::new(3)?)
    .rate_limiter(rate_limit_check)
    .build_checked();

assert!(pipeline.is_err()); // rate_limiter is inside retry
```

---

## Examples

### Minimal (retry only)

```rust
let pipeline = ResiliencePipeline::<MyError>::builder()
    .retry(RetryConfig::new(3)?.backoff(BackoffConfig::Fixed(Duration::from_millis(50))))
    .build();

let result = pipeline.call(|| Box::pin(async { Ok::<u32, MyError>(42) })).await;
```

### Full production stack

```rust
use nebula_resilience::{ResiliencePipeline, CallError};
use nebula_resilience::retry::{RetryConfig, BackoffConfig};
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use nebula_resilience::bulkhead::{Bulkhead, BulkheadConfig};
use nebula_resilience::rate_limiter::TokenBucket;
use std::sync::Arc;
use std::time::Duration;

let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig {
    failure_threshold: 5,
    reset_timeout: Duration::from_secs(30),
    ..Default::default()
})?);

let bh = Arc::new(Bulkhead::new(BulkheadConfig {
    max_concurrency: 20,
    ..Default::default()
})?);

let rate_limiter = Arc::new(TokenBucket::new(100, 50.0)?);

let pipeline = ResiliencePipeline::<reqwest::Error>::builder()
    .load_shed(Arc::new(|| false))
    .rate_limiter_from(rate_limiter)
    .timeout(Duration::from_secs(10))
    .retry(
        RetryConfig::new(3)?
            .backoff(BackoffConfig::exponential_default())
            .retry_if(|e: &reqwest::Error| e.is_timeout() || e.is_connect()),
    )
    .circuit_breaker(cb)
    .bulkhead(bh)
    .build();

let response = pipeline.call(|| Box::pin(async {
    http_client.get("https://api.example.com/data").send().await
})).await?;
```

### Sharing a pipeline across concurrent tasks

Wrap the pipeline in `Arc` when you want to share it across concurrent tasks:

```rust
let pipeline = Arc::new(
    ResiliencePipeline::<MyError>::builder()
        .timeout(Duration::from_secs(5))
        .build()
);

for _ in 0..16 {
    let pipeline = pipeline.clone();
    tokio::spawn(async move {
        let _ = pipeline.call(|| Box::pin(async { Ok::<_, MyError>(42) })).await;
    });
}
```

### Observing events with `RecordingSink` (testing)

```rust
use nebula_resilience::sink::{RecordingSink, ResilienceEventKind};
use nebula_resilience::PolicyScope;
use nebula_resilience::circuit_breaker::CircuitBreaker;
use std::sync::Arc;

let sink = RecordingSink::new();
let scope = PolicyScope::empty().tenant_id("tenant-a").operation("gmail.poll");
let cb = Arc::new(
    CircuitBreaker::new(CircuitBreakerConfig::default())?
        .with_sink(sink.clone())
);

let pipeline = ResiliencePipeline::<MyError>::builder()
    .with_sink(sink.clone())
    .scope(scope)
    .circuit_breaker(cb)
    .build_checked()?;

// ... run pipeline ...

assert!(sink.has_state_change(CircuitState::Open));
assert_eq!(sink.count(ResilienceEventKind::RetryAttempt), 3);
assert_eq!(sink.count(ResilienceEventKind::PipelineCompleted), 1);
```
