# API Reference (Human-Oriented)

## Public Surface

- **Stable APIs:** `ResilienceError`, `ResilienceResult`, `CircuitBreaker`, `RetryStrategy`, `Bulkhead`, `timeout`, `ResilienceManager`, `ResiliencePolicy`, `PolicyBuilder`, `LayerBuilder`, `ResilienceChain`, `Retryable`.
- **Experimental APIs:** typestate builder (`TypestatePolicyBuilder`), advanced retry conditions, dynamic config.
- **Hidden/internal:** `core::*` internals; pattern implementation details.

## Core Types

### `ResilienceError`

Rich error enum. Each variant carries structured context:

| Variant | Fields | `classify()` |
|---|---|---|
| `Timeout` | `duration`, `context: Option<String>` | `Transient` |
| `CircuitBreakerOpen` | `state: String`, `retry_after: Option<Duration>` | `ResourceExhaustion` |
| `BulkheadFull` | `max_concurrency`, `queued` | `ResourceExhaustion` |
| `RateLimitExceeded` | `retry_after`, `limit: f64`, `current: f64` | `ResourceExhaustion` |
| `RetryLimitExceeded` | `attempts`, `last_error: Option<Box<Self>>` | `Permanent` |
| `FallbackFailed` | `reason`, `original_error: Option<Box<Self>>` | `Permanent` |
| `Cancelled` | `reason: Option<String>` | `Permanent` |
| `InvalidConfig` | `message` | `Configuration` |
| `Custom` | `message`, `retryable: bool`, `source` | `Transient` or `Permanent` |

Helper constructors: `timeout(duration)`, `circuit_breaker_open(state)`, `bulkhead_full(max)`, `retry_limit_exceeded_with_cause(attempts, last)`, `custom(message)`.

Classification methods:
- `classify() → ErrorClass` — `Transient` | `ResourceExhaustion` | `Permanent` | `Configuration` | `Unknown`
- `is_retryable() → bool` — true for `Transient` and `ResourceExhaustion`
- `is_terminal() → bool` — true for `Permanent` and `Configuration`
- `retry_after() → Option<Duration>` — hint from `CircuitBreakerOpen` and `RateLimitExceeded`

### `ResilienceResult<T>`

Type alias for `Result<T, ResilienceError>`.

### Typed newtypes (`core::types`)

`RetryCount`, `RateLimit`, `Timeout`, `MaxConcurrency`, `FailureThreshold`, `DurationExt`, `ResilienceResultExt`.

## Pattern APIs

### Circuit breaker

```rust
// Const generics: FAILURE_THRESHOLD and RESET_TIMEOUT_MS validated at compile time
let config = CircuitBreakerConfig::<5, 30_000>::new()
    .with_half_open_limit(3)
    .with_min_operations(10);
let breaker = CircuitBreaker::new(config)?;
breaker.execute(|| async { Ok::<_, ResilienceError>("ok") }).await;
```

Preset aliases (type aliases for common `CircuitBreaker::<N, M>` configs):
- `StandardCircuitBreaker` — 5 failures, 30s reset
- `FastCircuitBreaker` — 3 failures, 10s reset
- `SlowCircuitBreaker` — 10 failures, 60s reset

Constructors: `standard_config()`, `fast_config()`, `slow_config()`.

State enum `CircuitState`: `Closed` | `Open` | `HalfOpen`.

### Retry

Const-generic retry strategy. `MAX_ATTEMPTS` is a compile-time constant:

```rust
let retry = exponential_retry::<3>()?;                    // ExponentialBackoff, 3 attempts
let retry = fixed_retry::<50, 2>()?;                      // FixedDelay 50ms, 2 attempts
let retry = aggressive_retry::<5>()?;                     // AggressiveCondition, 5 attempts
```

Custom config:

```rust
let config = RetryConfig::new(
    ExponentialBackoff::<100, 20, 5000>::default(),
    ConservativeCondition::<3>::new(),
).with_jitter(JitterPolicy::Equal);
let strategy = RetryStrategy::new(config)?;
let (result, stats) = strategy
        .execute(|| async { Ok::<_, ResilienceError>("ok") })
        .await
        .map_err(|failure| failure.error)?;
```

Error model:

- `execute(...)`, `execute_resilient(...)`, `execute_resilient_with_cancellation(...)` return
    `RetryExecutionResult<T, E> = Result<(T, RetryStats), RetryFailure<E>>`.
- `RetryFailure<E>` contains both `error` and `stats` so failed executions keep telemetry.
- Use `failure.into_parts()` to split `(error, stats)`.

Backoff policies: `ExponentialBackoff`, `FixedDelay`, `LinearBackoff`, `CustomBackoff`.

Jitter: `JitterPolicy::None` | `Full` | `Equal`.

Retry conditions: `ConservativeCondition<N>`, `AggressiveCondition<N>`, `TimeBasedCondition<MAX_MS>`.

`RetryStats` — attempt count, total elapsed, last delay.

### Timeout

```rust
let result = timeout(Duration::from_secs(10), async { Ok::<_, ResilienceError>("ok") }).await;
let result = timeout_with_original_error(Duration::from_secs(10), async { ... }).await;
```

### Bulkhead

```rust
let bulkhead = Bulkhead::new(BulkheadConfig {
    max_concurrency: 10,
    queue_size: 50,
    timeout: Some(Duration::from_secs(5)),
});
```

### Rate limiter

`TokenBucket`, `LeakyBucket`, `SlidingWindow`, `AdaptiveRateLimiter`, `AnyRateLimiter` (type-erased).

### Fallback / Hedge

- `FallbackStrategy`, `AnyStringFallbackStrategy`, `ValueFallback`
- `HedgeExecutor`, `HedgeConfig`
- `AdaptiveHedgeExecutor::with_target_percentile(percentile) -> ConfigResult<Self>`
    validates that percentile is finite and within `[0.0, 1.0]`.

## `Retryable` Trait

Lightweight bridge: domain errors implement this so resilience can inspect retryability without depending on domain crates.

```rust
pub trait Retryable: Error {
    fn is_retryable(&self) -> bool { true }          // default: retry everything
    fn retry_delay(&self) -> Duration { Duration::from_millis(100) }
    fn max_retries(&self) -> Option<u32> { None }    // None = use policy default
}
```

Blanket implementations provided:
- `std::io::Error` — retryable for `Interrupted`, `WouldBlock`, `TimedOut`, `ConnectionReset`, `ConnectionAborted`
- `std::fmt::Error` — never retryable

## Manager and Policies

### `ResilienceManager`

Centralized service policy registry and protected execution orchestration.

```rust
let manager = Arc::new(ResilienceManager::with_defaults());
manager.register_service("api", policy);
let result = manager.execute("api", "call", || async { Ok::<_, ResilienceError>(()) }).await;
```

- `register_service(name, policy)` — untyped string key
- `register_service_typed::<S>(policy)` — typed via `Service` trait (`S::NAME: &'static str`)
- `execute(service, operation, fut)` — applies full policy stack
- `execute_with_override(service, operation, policy_override, fut)` — per-call override

`register_service` semantics for existing services:
- validates incoming policy before mutating runtime state,
- rejects invalid updates as no-op (previous policy remains effective),
- removes runtime components absent in reloaded policy (`circuit_breaker`, `bulkhead`),
- preserves already-collected service metrics across reload.

### `ResiliencePolicy`

Serializable policy model (serde). Fields:

```rust
pub struct ResiliencePolicy {
    pub timeout: Option<Duration>,
    pub retry: Option<RetryPolicyConfig>,
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    pub bulkhead: Option<BulkheadConfig>,
    pub metadata: PolicyMetadata,
}
```

Named constructors:
- `ResiliencePolicy::new(name)` — empty policy with name
- `ResiliencePolicy::basic(timeout, retry_attempts)` — timeout + exponential retry
- `ResiliencePolicy::robust(timeout, retries, cb_config, bulkhead_config)` — full stack
- `ResiliencePolicy::microservice()` — 10s timeout, 3 retries, CB + bulkhead defaults

Builder methods: `with_timeout`, `with_retry`, `with_circuit_breaker`, `with_bulkhead`, `without_*`, `with_name`, `with_tag`, `with_priority`.

Utility methods: `is_enabled()`, `max_execution_time()`, `merge(other)`.

### `RetryPolicyConfig`

Serializable retry config (float-free — uses `multiplier_x10` for serde compatibility):

```rust
RetryPolicyConfig::exponential(max_attempts, base_delay)
RetryPolicyConfig::fixed(max_attempts, delay)
config.delay_for_attempt(attempt) -> Option<Duration>
```

### `PolicyBuilder`

Runtime builder for `ResiliencePolicy` via method chain.

### `PolicyMetadata`

`name`, `description`, `version`, `tags: Vec<String>`, `priority: u32`.

## Composition APIs

- `LayerBuilder<T>` — builds middleware-style stacks
- `ResilienceLayer<T>` — single composable layer
- `ResilienceChain<T>` — composed chain of layers

Used to compose `timeout → bulkhead → circuit → retry` in sequence.

## Observability

- `ObservabilityHook`, `LoggingHook`, `MetricsHook`, `ObservabilityHooks` — hook types
- `create_span`, `record_success`, `record_error` — span/event helpers in `observability::spans`

## Preset Utilities (`utils` module)

```rust
use nebula_resilience::utils;

let (breaker, retry) = utils::http_resilience()?;      // Standard CB + exponential retry x3
let (breaker, retry) = utils::realtime_resilience()?;  // Fast CB + fixed 50ms retry x2
let (breaker, retry) = utils::batch_resilience()?;     // Slow CB + aggressive retry x5
```

## Constants (`constants` module)

```rust
use nebula_resilience::constants;

constants::DEFAULT_TIMEOUT           // 30s
constants::DEFAULT_RETRY_ATTEMPTS    // 3
constants::DEFAULT_FAILURE_THRESHOLD // 5
constants::DEFAULT_RATE_LIMIT        // 100.0 req/s
```

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
        retry
            .execute_resilient(|| async { Ok::<_, ResilienceError>("ok") })
            .await
            .map_err(|failure| failure.error)
    }).await;
    Ok(())
}
```

## Advanced Example

```rust
use nebula_resilience::prelude::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let policy = ResiliencePolicy::robust(
        Duration::from_secs(10),
        3,
        CircuitBreakerConfig::default(),
        BulkheadConfig::default(),
    );
    let manager = Arc::new(ResilienceManager::with_defaults());
    manager.register_service("api", policy);
    let result = manager
        .execute("api", "call", || async { Ok::<_, ResilienceError>(()) })
        .await;
    Ok(())
}
```

## Error Semantics

- **Retryable** (`is_retryable() == true`): `Timeout`, `RateLimitExceeded`, `CircuitBreakerOpen`, `Custom { retryable: true }`, `BulkheadFull`.
- **Terminal** (`is_terminal() == true`): `RetryLimitExceeded`, `FallbackFailed`, `Cancelled`, `InvalidConfig`, `Custom { retryable: false }`.

## Failure Defaults

- **Default stance:** fail-closed for `timeout`, `bulkhead`, `rate_limiter`, and `circuit_breaker`.
- **Conditional:** `retry` remains fail-closed at budget/terminal boundaries.
- **Opt-in fail-open:** `fallback` and `hedge` provide graceful degradation only when explicitly configured.

For consolidated operational defaults/limits and tuning guidance for `governor`, `timeout`, `fallback`, and `hedge`, see `RELIABILITY.md` (`Consolidated Pattern Defaults and Limits`).

## Compatibility Rules

- **Major version bump:** policy schema change; pattern order contract; cancellation semantics.
- **Deprecation policy:** 2 minor versions before removal; `#[deprecated]` with migration path.
