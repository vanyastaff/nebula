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
timeout → retry → circuit_breaker → bulkhead
```

Why this order:
- `timeout` as outermost enforces a **single deadline across all retry attempts**.
- `retry` sits inside timeout so each attempt consumes from the same budget.
- `circuit_breaker` is checked per attempt — a tripped breaker stops retrying early
  via the bail mechanism (non-Operation errors are not retried).
- `bulkhead` is innermost — concurrency is capped per individual attempt.

> **Note**: placing `timeout` *inside* `retry` gives each attempt its own independent
> deadline. `build()` emits a `tracing::warn!` if this ordering is detected.

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

let bh = Arc::new(Bulkhead::new(20));

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

    /// Add a circuit breaker step. Takes Arc so it can be shared / inspected externally.
    #[must_use]
    pub fn circuit_breaker(self, cb: Arc<CircuitBreaker>) -> Self

    /// Add a bulkhead step. Takes Arc so it can be shared.
    #[must_use]
    pub fn bulkhead(self, bh: Arc<Bulkhead>) -> Self

    /// Consume the builder and return the pipeline.
    /// Emits tracing::warn! if timeout is inside retry.
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
}
```

`call()` accepts a factory closure `F` (not a single future) because some steps (retry,
circuit breaker) must be able to invoke the operation multiple times.

---

## Step Interactions

### Retry and non-Operation errors

The retry step uses a **bail mechanism**: if an inner step returns a non-Operation
`CallError` (e.g. `CircuitOpen`, `BulkheadFull`), the error is stashed and retrying
stops immediately. This avoids hammering a tripped circuit breaker with rapid retries.

```
retry step receives:
  Ok(v)              → return Ok(v)
  Err(Operation(e))  → retry_if(e)? sleep, try again : return Err(Operation(e))
  Err(CircuitOpen)   → stop retrying, return Err(CircuitOpen)
  Err(BulkheadFull)  → stop retrying, return Err(BulkheadFull)
  Err(Timeout)       → stop retrying, return Err(Timeout)
```

### `CircuitBreaker` and `Bulkhead` unwrapping

These steps call the remaining pipeline through an `unwrap_inner` shim that maps
`Ok(v)` and `Err(Operation(e))` to the `Result<T, E>` their `.call()` methods expect.
Any other `CallError` variant inside a `CircuitBreaker` or `Bulkhead` step is
**unreachable** in a correctly ordered pipeline (timeout and retry must be outside).

---

## Layer Order Warning

`build()` inspects step positions and warns via `tracing::warn!` if timeout appears
after retry in the step list:

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
use nebula_resilience::bulkhead::Bulkhead;
use std::sync::Arc;
use std::time::Duration;

let cb = Arc::new(CircuitBreaker::new(CircuitBreakerConfig {
    failure_threshold: 5,
    reset_timeout: Duration::from_secs(30),
    ..Default::default()
})?);

let bh = Arc::new(Bulkhead::new(20));

let pipeline = ResiliencePipeline::<reqwest::Error>::builder()
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

`ResiliencePipeline` is `Clone` (internal state is `Arc`):

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
use nebula_resilience::sink::RecordingSink;
use nebula_resilience::circuit_breaker::CircuitBreaker;
use std::sync::Arc;

let sink = Arc::new(RecordingSink::new());
let cb = Arc::new(
    CircuitBreaker::with_sink(CircuitBreakerConfig::default(), sink.clone())?
);

// ... run pipeline ...

assert!(sink.has_state_change(CircuitState::Open));
assert_eq!(sink.count("retry_attempt"), 3);
```
