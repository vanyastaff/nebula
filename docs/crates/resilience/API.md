# API Reference (Human-Oriented)

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
