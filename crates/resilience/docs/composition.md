# nebula-resilience — Composition

`LayerBuilder` and `ResilienceChain` let you compose individual resilience patterns into
an ordered middleware pipeline without writing boilerplate. Each layer wraps the next;
the innermost layer calls the actual operation.

---

## Table of Contents

- [Recommended Layer Order](#recommended-layer-order)
- [LayerBuilder API](#layerbuilder-api)
- [ResilienceChain Execution](#resiliencechain-execution)
- [Built-in Layers](#built-in-layers)
- [Custom Layers](#custom-layers)
- [Cancellation Propagation](#cancellation-propagation)
- [Layer Internals](#layer-internals)
- [Examples](#examples)

---

## Recommended Layer Order

When composing patterns, add them to `LayerBuilder` in this order:

```
with_timeout → with_bulkhead → with_circuit_breaker → with_retry
```

Why this order:
- `timeout` enforces the total budget from the outermost position.
- `bulkhead` rejects immediately when at capacity, before spending circuit-breaker state.
- `circuit_breaker` fails-fast when the dependency is unhealthy.
- `retry` is innermost so each attempt is individually circuit-checked and bulkhead-counted.

See [PATTERNS.md](PATTERNS.md) for the full rationale.

---

## LayerBuilder API

`LayerBuilder<T>` is generic over the successful return type `T`. All builder methods
return `Self` and are marked `#[must_use]`.

```rust
use nebula_resilience::compose::LayerBuilder;
use std::sync::Arc;
use std::time::Duration;

let chain = LayerBuilder::new()
    .with_timeout(Duration::from_secs(5))
    .with_bulkhead(Arc::new(bulkhead))
    .with_circuit_breaker(Arc::new(breaker))
    .with_retry_exponential(3, Duration::from_millis(100))
    .build();
```

### Builder methods

```rust
impl<T: Send + 'static> LayerBuilder<T> {
    pub fn new() -> Self

    // Add a hard deadline.
    pub fn with_timeout(self, duration: Duration) -> Self

    // Add retry with a pre-built RetryPolicyConfig.
    pub fn with_retry(self, config: RetryPolicyConfig) -> Self

    // Add retry with exponential backoff (shorthand).
    pub fn with_retry_exponential(self, max_attempts: usize, base_delay: Duration) -> Self

    // Add retry with fixed delay (shorthand).
    pub fn with_retry_fixed(self, max_attempts: usize, delay: Duration) -> Self

    // Add an Arc<CircuitBreaker> as a layer.
    pub fn with_circuit_breaker(self, circuit_breaker: Arc<CircuitBreaker>) -> Self

    // Add an Arc<Bulkhead> as a layer.
    pub fn with_bulkhead(self, bulkhead: Arc<Bulkhead>) -> Self

    // Add any custom ResilienceLayer implementation.
    pub fn with_layer(self, layer: Arc<dyn ResilienceLayer<T> + Send + Sync>) -> Self

    // Consume the builder and return an Arc<dyn LayerStack<T>>.
    pub fn build(self) -> ResilienceChain<T>
}
```

### `RetryPolicyConfig` shorthand constructors

When using `with_retry()` directly:

```rust
use nebula_resilience::policy::RetryPolicyConfig;
use std::time::Duration;

// Exponential: base 100ms, 2x multiplier, 30s cap, with jitter
let config = RetryPolicyConfig::exponential(3, Duration::from_millis(100));

// Fixed: always 200ms, no jitter
let config = RetryPolicyConfig::fixed(5, Duration::from_millis(200));
```

---

## ResilienceChain Execution

`build()` returns a `ResilienceChain<T>`:

```rust
pub type ResilienceChain<T> = Arc<dyn LayerStack<T> + Send + Sync>;
```

Execution via the `LayerStack<T>` trait:

```rust
// Without cancellation
let result: ResilienceResult<T> = chain.execute(&boxed_operation).await;

// With cooperative cancellation
let result: ResilienceResult<T> = chain
    .execute_with_cancellation(&boxed_operation, Some(&cancellation_context))
    .await;
```

For closure-based callers, wrap with `BoxedOperation`:

```rust
use nebula_resilience::compose::BoxedOperation;

let op = BoxedOperation::new(move || async { Ok::<_, ResilienceError>("result") });
let result = chain.execute(&op).await;
```

---

## Built-in Layers

### `TimeoutLayer`

Wraps inner execution in `tokio::time::timeout`. Returns `ResilienceError::Timeout`
with the configured duration on expiry.

### `RetryLayer`

Reads `RetryPolicyConfig.delay_for_attempt(attempt)` to compute per-attempt delay.
Checks `ResilienceError::is_retryable()` to decide whether to continue. Stops when
attempt count reaches `max_attempts` or the error is non-retryable, returning
`ResilienceError::RetryLimitExceeded`.

Respects cooperative cancellation: while sleeping between attempts, if the
`CancellationContext` token fires, the layer returns `ResilienceError::Cancelled`
immediately via `tokio::select!`.

### `CircuitBreakerLayer`

Calls `circuit_breaker.can_execute()` before delegating. Records success or failure
after each attempt. Works with any `CircuitBreaker<N, M>` (type-erased via `Arc`).

### `BulkheadLayer`

Calls `bulkhead.acquire()` to take a semaphore permit before delegating. The permit
is released via RAII after the inner layer returns — whether success or error.

---

## Custom Layers

Implement `ResilienceLayer<T>` to add your own middleware:

```rust
use async_trait::async_trait;
use nebula_resilience::compose::{BoxedOperation, LayerStack, ResilienceLayer};
use nebula_resilience::{ResilienceError, ResilienceResult};
use nebula_resilience::core::CancellationContext;

pub struct LoggingLayer {
    operation_name: &'static str,
}

#[async_trait]
impl<T: Send + 'static> ResilienceLayer<T> for LoggingLayer {
    async fn apply(
        &self,
        operation: &BoxedOperation<T>,
        next: &(dyn LayerStack<T> + Send + Sync),
        cancellation: Option<&CancellationContext>,
    ) -> ResilienceResult<T> {
        tracing::info!(op = self.operation_name, "starting");
        let result = next.execute_with_cancellation(operation, cancellation).await;
        match &result {
            Ok(_) => tracing::info!(op = self.operation_name, "succeeded"),
            Err(e) => tracing::warn!(op = self.operation_name, error = %e, "failed"),
        }
        result
    }

    fn name(&self) -> &'static str {
        "logging"
    }
}

// Register with the builder:
let chain = LayerBuilder::new()
    .with_layer(Arc::new(LoggingLayer { operation_name: "db_query" }))
    .with_retry_exponential(3, Duration::from_millis(50))
    .build();
```

---

## Cancellation Propagation

Every layer receives an `Option<&CancellationContext>` and must propagate it to
`next.execute_with_cancellation(op, cancellation)`. Built-in layers all forward the
context. If a cancellation fires:

1. `RetryLayer` cancels the inter-attempt sleep and returns `ResilienceError::Cancelled`.
2. `TerminalLayer` (innermost) checks the token before executing and returns
   `ResilienceError::Cancelled` if already cancelled.
3. `TimeoutLayer` races the timeout future against the inner execution; a cancelled
   inner future races against the timeout as normal.

Obtain a `CancellationContext` from a `tokio_util::sync::CancellationToken`:

```rust
use nebula_resilience::core::CancellationContext;
use tokio_util::sync::CancellationToken;

let token = CancellationToken::new();
let ctx = CancellationContext::from_token(token.clone());

// On shutdown:
token.cancel();
```

---

## Layer Internals

The stack is built by `LayerBuilder::build()` in reverse order so that layers added
first are outermost:

```rust
let mut stack: Arc<dyn LayerStack<T>> = Arc::new(TerminalLayer);

for layer in self.layers.into_iter().rev() {
    stack = Arc::new(ComposedStack { layer, next: stack });
}
```

`ComposedStack` delegates to `layer.apply(op, next, cancellation)`, which in turn
forwards to the remaining `next` stack. `TerminalLayer` is always the last element and
calls `BoxedOperation::execute()` directly.

---

## Examples

### HTTP client with full production stack

```rust
use nebula_resilience::{
    CircuitBreaker, CircuitBreakerConfig, ResilienceError,
    patterns::bulkhead::{Bulkhead, BulkheadConfig},
};
use nebula_resilience::compose::LayerBuilder;
use std::sync::Arc;
use std::time::Duration;

let breaker = Arc::new(
    CircuitBreaker::new(CircuitBreakerConfig::<5, 30_000>::new())?,
);

let bulkhead = Arc::new(Bulkhead::with_config(BulkheadConfig {
    max_concurrency: 20,
    ..Default::default()
}));

let chain = LayerBuilder::new()
    .with_timeout(Duration::from_secs(10))
    .with_bulkhead(bulkhead)
    .with_circuit_breaker(breaker)
    .with_retry_exponential(3, Duration::from_millis(200))
    .build();

let response = chain.execute(|| async {
    // Perform HTTP request
    Ok::<_, ResilienceError>(http_client.get("https://api.example.com/data").await?)
}).await?;
```

### Sharing a chain arc across concurrent tasks

`ResilienceChain<T>` is `Arc<dyn LayerStack<T>>`. Clone the arc to share across tasks:

```rust
let chain = Arc::new(/* ... */);

for _ in 0..16 {
    let chain = chain.clone();
    tokio::spawn(async move {
        let _ = chain.execute(|| async { Ok::<_, ResilienceError>(42) }).await;
    });
}
```

### Minimal chain (retry only)

```rust
let chain = LayerBuilder::<String>::new()
    .with_retry_fixed(3, Duration::from_millis(50))
    .build();
```
