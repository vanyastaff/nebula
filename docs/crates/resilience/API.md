# API Reference (Human-Oriented)

## Public Surface

- **Stable APIs:** `ResilienceError`, `ResilienceResult`, `CircuitBreaker`, `RetryStrategy`, `Bulkhead`, `timeout`, `ResilienceManager`, `ResiliencePolicy`, `PolicyBuilder`, `LayerBuilder`, `ResilienceChain`, `Retryable`.
- **Experimental APIs:** typestate builder (`TypestatePolicyBuilder`), advanced retry conditions, dynamic config.
- **Hidden/internal:** `core::*` internals; pattern implementation details.

## Core Types

- `ResilienceError`
- `ResilienceResult<T>`
- config/result helpers from `core::config` and `core::result`
- typed newtypes from `core::types` (`RetryCount`, `RateLimit`, `Timeout`, etc.)

## Pattern APIs

- Circuit breaker:
  - `CircuitBreaker`
  - `CircuitBreakerConfig`
  - `CircuitState`
- Retry:
  - `RetryStrategy`
  - `RetryConfig`
  - backoff policies (`ExponentialBackoff`, `FixedDelay`, `LinearBackoff`, ...)
  - helper constructors (`exponential_retry`, `fixed_retry`, `aggressive_retry`)
- Timeout:
  - `timeout(...)`
  - `timeout_with_original_error(...)`
- Bulkhead:
  - `Bulkhead`
  - `BulkheadConfig`
- Rate limiter:
  - `TokenBucket`, `LeakyBucket`, `SlidingWindow`, `AdaptiveRateLimiter`, `GovernorRateLimiter`
- Fallback/Hedge:
  - `FallbackStrategy` family
  - `HedgeExecutor`, `HedgeConfig`

## Manager and Policies

- `ResilienceManager`
  - `register_service`, `execute`, `execute_with_override`
  - typed helpers: `register_service_typed`, `execute_typed`, metrics accessors
- `PolicyBuilder`
- `ResiliencePolicy`
- `RetryPolicyConfig`
- `PolicyMetadata`

## Composition APIs

- `LayerBuilder<T>`
- `ResilienceLayer<T>`
- `LayerStack<T>`
- `ResilienceChain<T>`

Used for composing timeout/retry/circuit-breaker/bulkhead layers in middleware style.

## Observability

- hooks:
  - `ObservabilityHook`, `LoggingHook`, `MetricsHook`, `ObservabilityHooks`
- spans/events helpers:
  - `create_span`, `record_success`, `record_error`
  - event categories and metric helpers in `observability::hooks`

## Retryable Bridge

- `retryable::Retryable`
  - lightweight trait for domain errors to declare retryability and delay hints

## Usage Patterns

- **Standalone pattern:** use `CircuitBreaker::execute`, `RetryStrategy::execute_resilient`, `timeout()` directly.
- **Manager-based:** register services with `ResilienceManager`, call `execute` with service name.
- **Composition:** use `LayerBuilder` or `ResilienceChain` to stack timeout → bulkhead → circuit → retry.
- **Config-driven:** load `ResiliencePolicy` from `nebula-config`; build manager from policy.

## Minimal Example

```rust
use nebula_resilience::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let breaker = StandardCircuitBreaker::default();
    let retry = exponential_retry::<3>()?;
    let result = breaker.execute(|| async {
        retry.execute_resilient(|| async { Ok::<_, ResilienceError>("ok") }).await
    }).await;
    Ok(())
}
```

## Advanced Example

```rust
use nebula_resilience::prelude::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let policy = PolicyBuilder::new()
        .with_timeout(Duration::from_secs(10))
        .with_retry_exponential(3, Duration::from_millis(100))
        .with_circuit_breaker(CircuitBreakerConfig::<5, 30_000>::new())
        .with_bulkhead(BulkheadConfig {
            max_concurrency: 10,
            queue_size: 50,
            timeout: Some(Duration::from_secs(10)),
        })
        .build();
    let manager = Arc::new(ResilienceManager::with_defaults());
    manager.register_service("api", policy);
    let result = manager
        .execute("api", "call", || async {
            // external HTTP call
            Ok::<_, ResilienceError>(())
        })
        .await;
    Ok(())
}
```

## Error Semantics

- **Retryable errors:** `Timeout`, `RateLimitExceeded`, `CircuitBreakerOpen` (when `retry_after` set), `Custom { retryable: true }`.
- **Fatal errors:** `RetryLimitExceeded`, `FallbackFailed`, `Cancelled`, `InvalidConfig`, `Custom { retryable: false }`.
- **Validation errors:** `ConfigError` from policy/build validation.

## Compatibility Rules

- **Major version bump:** policy schema change; pattern order contract; cancellation semantics.
- **Deprecation policy:** 2 minor versions before removal; `#[deprecated]` with migration path.
