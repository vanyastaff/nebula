# nebula-resilience — API Reference

Complete public API reference. All types live in `nebula_resilience` unless noted.

---

## Table of Contents

- [Core Types](#core-types)
- [Circuit Breaker](#circuit-breaker)
- [Retry](#retry)
- [Bulkhead](#bulkhead)
- [Rate Limiter](#rate-limiter)
- [Timeout](#timeout)
- [Fallback](#fallback)
- [Hedge](#hedge)
- [Gate](#gate)
- [Manager](#manager)
- [Policy](#policy)
- [Composition](#composition)
- [Observability](#observability)
- [Prelude](#prelude)

---

## Core Types

### `ResilienceError`

Rich error enum. Every variant is `#[non_exhaustive]`.

```rust
#[non_exhaustive]
pub enum ResilienceError {
    Timeout {
        duration: Duration,
        context: Option<String>,
    },
    CircuitBreakerOpen {
        state: String,
        retry_after: Option<Duration>,
    },
    BulkheadFull {
        max_concurrency: usize,
        queued: usize,
    },
    RateLimitExceeded {
        retry_after: Option<Duration>,
        limit: f64,
        current: f64,
    },
    RetryLimitExceeded {
        attempts: usize,
        last_error: Option<Box<ResilienceError>>,
    },
    FallbackFailed {
        reason: String,
        original_error: Option<Box<ResilienceError>>,
    },
    Cancelled {
        reason: Option<String>,
    },
    InvalidConfig {
        message: String,
    },
    Custom {
        message: String,
        retryable: bool,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}
```

**Factory constructors:**

```rust
ResilienceError::timeout(duration: Duration) -> Self
ResilienceError::circuit_breaker_open(state: impl Into<String>) -> Self
ResilienceError::bulkhead_full(max: usize) -> Self
ResilienceError::rate_limit_exceeded(retry_after: Option<Duration>) -> Self
ResilienceError::retry_limit_exceeded(attempts: usize) -> Self
ResilienceError::retry_limit_exceeded_with_cause(attempts: usize, last: ResilienceError) -> Self
ResilienceError::cancelled() -> Self
ResilienceError::invalid_config(message: impl Into<String>) -> Self
ResilienceError::custom(message: impl Into<String>) -> Self
```

**Classification methods:**

| Method | Return | Notes |
|--------|--------|-------|
| `classify()` | `ErrorClass` | `Transient` \| `ResourceExhaustion` \| `Permanent` \| `Configuration` \| `Unknown` |
| `is_retryable()` | `bool` | `true` for `Transient` and `ResourceExhaustion` |
| `is_terminal()` | `bool` | `true` for `Permanent` and `Configuration` |
| `retry_after()` | `Option<Duration>` | Hint from `CircuitBreakerOpen` and `RateLimitExceeded` |

---

### `ResilienceResult<T>`

```rust
pub type ResilienceResult<T> = Result<T, ResilienceError>;
```

---

### `ErrorClass`

```rust
pub enum ErrorClass {
    Transient,
    ResourceExhaustion,
    Permanent,
    Configuration,
    Unknown,
}
```

---

### `Retryable`

```rust
pub trait Retryable {
    fn is_retryable(&self) -> bool;
}
```

Blanket impls are provided for `std::io::Error`, and optionally for `reqwest::Error`
and `sqlx::Error` via feature flags.

---

## Circuit Breaker

### `CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>`

```rust
pub struct CircuitBreakerConfig<
    const FAILURE_THRESHOLD: usize = 5,
    const RESET_TIMEOUT_MS: u64 = 30_000,
> {
    pub half_open_max_operations: usize,  // default: 3
    pub count_timeouts: bool,             // default: true
    pub min_operations: usize,            // default: 10
    pub failure_rate_threshold: f64,      // default: 0.6
    pub sliding_window_ms: u64,          // default: 60_000
}
```

Builder methods (each returns `Self`, all `#[must_use]`):

```rust
config.with_half_open_limit(n: usize) -> Self
config.with_min_operations(n: usize) -> Self
config.with_count_timeouts(v: bool) -> Self
config.with_failure_rate_threshold(rate: f64) -> Self
config.with_sliding_window(ms: u64) -> Self
```

Preset constructors:

```rust
CircuitBreakerConfig::standard_config()  // 5 failures, 30 s
CircuitBreakerConfig::fast_config()      // 3 failures, 10 s
CircuitBreakerConfig::slow_config()      // 10 failures, 60 s
```

---

### `CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>`

```rust
impl<const F: usize, const R: u64> CircuitBreaker<F, R> {
    pub fn new(config: CircuitBreakerConfig<F, R>) -> ResilienceResult<Self>;

    pub async fn execute<T, E, Fut, FutFn>(
        &self,
        operation: FutFn,
    ) -> Result<T, E>
    where
        FutFn: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: From<ResilienceError>;

    pub fn state(&self) -> CircuitState;
    pub fn stats(&self) -> CircuitBreakerStats;
    pub fn reset(&self);
}
```

### `CircuitState`

```rust
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}
```

### `CircuitBreakerStats`

```rust
pub struct CircuitBreakerStats {
    pub state: CircuitState,
    pub failure_count: usize,
    pub success_count: usize,
    pub last_failure_time: Option<Instant>,
    pub last_state_change: Instant,
}
```

Preset type aliases:

```rust
pub type StandardCircuitBreaker = CircuitBreaker<5, 30_000>;
pub type FastCircuitBreaker     = CircuitBreaker<3, 10_000>;
pub type SlowCircuitBreaker     = CircuitBreaker<10, 60_000>;
```

---

## Retry

### Backoff policies

All policies implement `BackoffPolicy` (sealed):

```rust
pub trait BackoffPolicy: Send + Sync + 'static {
    fn calculate_delay(&self, attempt: usize) -> Duration;
    fn max_delay(&self) -> Duration;
    fn policy_name(&self) -> &'static str;
}
```

| Type | Const generics | Delay formula |
|------|---------------|---------------|
| `FixedDelay<DELAY_MS>` | `DELAY_MS: u64` | `DELAY_MS` always |
| `ExponentialBackoff<BASE_MS, MULTIPLIER_X10, MAX_MS>` | all `u64` | `base * (mult/10)^attempt`, capped at `MAX_MS` |
| `LinearBackoff<STEP_MS, MAX_MS>` | all `u64` | `STEP_MS * attempt`, capped at `MAX_MS` |
| `CustomBackoff` | — | user-provided `fn(usize) -> Duration` |

---

### Retry conditions

All conditions implement `RetryCondition` (sealed):

| Type | Const generics | Behaviour |
|------|---------------|-----------|
| `ConservativeCondition<N>` | `N: usize` | retry only `Transient` errors; stop after `N` attempts |
| `AggressiveCondition<N>` | `N: usize` | retry `Transient` and `ResourceExhaustion`; stop after `N` |
| `TimeBasedCondition<MAX_MS>` | `MAX_MS: u64` | retry until cumulative elapsed exceeds `MAX_MS` |

---

### `JitterPolicy`

```rust
pub enum JitterPolicy {
    None,
    Full,   // random in [0, delay]
    Equal,  // delay/2 + random in [0, delay/2]
}
```

---

### `RetryConfig<B, C>`

```rust
pub struct RetryConfig<B: BackoffPolicy, C: RetryCondition> {
    backoff: B,
    condition: C,
    jitter: JitterPolicy,
}

impl<B, C> RetryConfig<B, C> {
    pub fn new(backoff: B, condition: C) -> Self;
    pub fn with_jitter(self, policy: JitterPolicy) -> Self;
}
```

---

### `RetryStrategy<B, C, MAX_ATTEMPTS>`

```rust
impl<B, C, const MAX_ATTEMPTS: usize> RetryStrategy<B, C, MAX_ATTEMPTS> {
    pub fn new(config: RetryConfig<B, C>) -> ResilienceResult<Self>;

    /// Execute and return (value, stats) on success, RetryFailure on exhaustion.
    pub async fn execute<T, E, Fut, FutFn>(
        &self,
        operation: FutFn,
    ) -> RetryExecutionResult<T, E>
    where
        FutFn: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: Retryable + Clone;

    /// Same as execute() with a ResilienceError error type.
    pub async fn execute_resilient<T, Fut, FutFn>(
        &self,
        operation: FutFn,
    ) -> RetryExecutionResult<T, ResilienceError>
    where …;

    /// Same but honours cooperative cancellation via CancellationContext.
    pub async fn execute_resilient_with_cancellation<T, Fut, FutFn>(
        &self,
        operation: FutFn,
        cancellation: &CancellationContext,
    ) -> RetryExecutionResult<T, ResilienceError>
    where …;

    pub fn stats(&self) -> &RetryStats;
}

pub type RetryExecutionResult<T, E> = Result<(T, RetryStats), RetryFailure<E>>;
```

---

### `RetryStats`

```rust
pub struct RetryStats {
    pub attempts: usize,
    pub total_duration: Duration,
    pub delays: Vec<Duration>,
}
```

---

### `RetryFailure<E>`

```rust
pub struct RetryFailure<E> {
    pub error: E,
    pub stats: RetryStats,
}

impl<E> RetryFailure<E> {
    pub fn into_parts(self) -> (E, RetryStats);
}
```

---

### Convenience constructors

```rust
// ExponentialBackoff, ConservativeCondition<N>
pub fn exponential_retry<const N: usize>() -> ResilienceResult<RetryStrategy<…>>;

// FixedDelay<DELAY_MS>, ConservativeCondition<N>
pub fn fixed_retry<const DELAY_MS: u64, const N: usize>() -> ResilienceResult<RetryStrategy<…>>;

// ExponentialBackoff, AggressiveCondition<N>
pub fn aggressive_retry<const N: usize>() -> ResilienceResult<RetryStrategy<…>>;
```

---

## Bulkhead

### `BulkheadConfig`

```rust
pub struct BulkheadConfig {
    pub max_concurrency: usize,           // default: 10
    pub queue_size: usize,                // default: 100
    pub timeout: Option<Duration>,        // default: Some(30s)
}
```

---

### `Bulkhead`

```rust
impl Bulkhead {
    pub fn new(max_concurrency: usize) -> Self;
    pub fn with_config(config: BulkheadConfig) -> Self;

    pub async fn execute<T, E, Fut, FutFn>(
        &self,
        operation: FutFn,
    ) -> Result<T, E>
    where
        FutFn: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: From<ResilienceError>;

    pub fn try_execute<T, E, Fut, FutFn>(&self, operation: FutFn) -> Option<…>;

    pub fn active_operations(&self) -> usize;
    pub fn available_permits(&self) -> usize;
    pub fn max_concurrency(&self) -> usize;
    pub fn is_at_capacity(&self) -> bool;
    pub fn stats(&self) -> BulkheadStats;
}
```

### `BulkheadStats`

```rust
pub struct BulkheadStats {
    pub active: usize,
    pub waiting: usize,
    pub available: usize,
    pub max_concurrency: usize,
}
```

---

## Rate Limiter

```rust
pub struct RateLimiterConfig {
    pub rate: f64,              // tokens per second
    pub burst: usize,           // maximum token bucket size
    pub window: Duration,       // replenishment window
}

impl RateLimiter {
    pub fn new(config: RateLimiterConfig) -> ResilienceResult<Self>;

    pub async fn execute<T, E, Fut, FutFn>(
        &self,
        operation: FutFn,
    ) -> Result<T, E>
    where
        FutFn: Fn() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        E: From<ResilienceError>;

    pub fn try_execute<T, E, Fut, FutFn>(&self, operation: FutFn) -> Option<…>;
    pub fn available_tokens(&self) -> f64;
}
```

---

## Timeout

```rust
/// Applies a hard deadline to an async future.
///
/// Returns `ResilienceError::Timeout` if the future does not complete within
/// `duration`. Otherwise returns the inner result unchanged.
pub async fn timeout<T, E, Fut>(
    duration: Duration,
    future: Fut,
) -> Result<T, E>
where
    Fut: Future<Output = Result<T, E>>,
    E: From<ResilienceError>;
```

---

## Fallback

### `FallbackStrategy<T>`

```rust
#[async_trait]
pub trait FallbackStrategy<T>: Send + Sync {
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<T>;

    /// Default: fallback for all errors except `InvalidConfig`.
    fn should_fallback(&self, error: &ResilienceError) -> bool { … }
}
```

### Built-in implementations

| Type | Behaviour |
|------|-----------|
| `ValueFallback<T>` | Returns a cloned constant value. |
| `FunctionFallback<T, F, Fut>` | Calls a closure `F: Fn(ResilienceError) -> Fut`. |
| `CacheFallback<T>` | Returns last cached successful value within a TTL. |
| `ChainedFallback<T>` | Tries strategies in order; returns first success. |

```rust
// ValueFallback
let fallback = ValueFallback::new("default".to_string());

// FunctionFallback
let fallback = FunctionFallback::new(|err| async move {
    tracing::warn!(%err, "falling back");
    Ok("degraded".to_string())
});

// CacheFallback
let fallback = CacheFallback::<String>::new(Duration::from_secs(60));
fallback.update("last known good".to_string()).await;

// ChainedFallback
let fallback = ChainedFallback::new(vec![
    Arc::new(ValueFallback::new("static".to_string())),
]);
```

### Top-level helper

```rust
pub async fn execute_with_fallback<T, Fut, FutFn, FS>(
    operation: FutFn,
    fallback: &FS,
) -> ResilienceResult<T>
where
    FutFn: Fn() -> Fut,
    Fut: Future<Output = ResilienceResult<T>>,
    FS: FallbackStrategy<T>;
```

---

## Hedge

### `HedgeConfig`

```rust
pub struct HedgeConfig {
    pub hedge_delay: Duration,       // default: 50 ms
    pub max_hedges: usize,           // default: 2
    pub exponential_backoff: bool,   // default: true
    pub backoff_multiplier: f64,     // default: 2.0
}
```

### `HedgeExecutor`

```rust
impl HedgeExecutor {
    pub fn new(config: HedgeConfig) -> Self;

    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send;
}
```

### `AdaptiveHedgeExecutor`

Adjusts `hedge_delay` based on observed p50/p95 latency statistics. Activates hedging
after a `warmup_requests` window. Configuration:

```rust
pub struct AdaptiveHedgeConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub min_delay: Duration,
    pub percentile: f64,         // e.g. 0.95
    pub warmup_requests: usize,
}
```

---

## Gate

See [gate.md](gate.md) for full usage guide.

```rust
pub struct Gate { /* ... */ }
pub struct GateGuard { /* ... */ }

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("gate is closed — new enter() calls are rejected")]
pub struct GateClosed;

impl Gate {
    pub fn new() -> Self;
    pub fn enter(&self) -> Result<GateGuard, GateClosed>;
    pub async fn close(&self);
    pub async fn close_with_timeout(&self, timeout: Duration) -> bool;
    pub fn is_closed(&self) -> bool;
}

impl Drop for GateGuard {
    fn drop(&mut self); // returns one permit to the semaphore
}
```

---

## Manager

### `Service` trait

```rust
pub trait Service: Send + Sync + 'static {
    const NAME: &'static str;
    type Category: ServiceCategory;

    fn name() -> &'static str { Self::NAME }
    fn category_name() -> &'static str { Self::Category::name() }
}
```

### `Operation` trait

```rust
pub trait Operation: Send + Sync + 'static {
    const NAME: &'static str;
    const IDEMPOTENT: bool = false;
}
```

### `RetryableOperation<T>`

Object-safe trait used internally for type erasure:

```rust
#[async_trait]
pub trait RetryableOperation<T>: Send + Sync {
    async fn execute(&self) -> ResilienceResult<T>;
}
```

### `ResilienceManager`

```rust
impl ResilienceManager {
    pub fn new() -> Self;
    pub fn with_config(config: ResilienceConfig) -> Self;

    /// Register a named policy for service S.
    pub async fn register_service<S: Service>(&self, policy: ResiliencePolicy);

    /// Execute operation for typed service S.
    pub async fn execute_typed<S, T, E, Fut, FutFn>(
        &self,
        operation: FutFn,
    ) -> Result<T, E>
    where
        S: Service,
        FutFn: Fn() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        E: From<ResilienceError> + Send;

    /// Execute operation for dynamically named service.
    pub async fn execute<T, E, Fut, FutFn>(
        &self,
        service_name: &str,
        operation: FutFn,
    ) -> Result<T, E>
    where …;

    /// Execute with cooperative cancellation.
    pub async fn execute_with_cancellation<T, E, Fut, FutFn>(
        &self,
        service_name: &str,
        operation: FutFn,
        cancellation: &CancellationContext,
    ) -> Result<T, E>
    where …;

    pub fn service_count(&self) -> usize;
    pub fn total_executions(&self) -> u64;
}
```

---

## Policy

### `RetryPolicyConfig`

Serialisable retry configuration (does not use const generics — suitable for dynamic
config loading):

```rust
pub struct RetryPolicyConfig {
    pub max_attempts: usize,      // default: 3
    pub base_delay_ms: u64,       // default: 100
    pub max_delay_ms: u64,        // default: 30_000
    pub multiplier_x10: u64,      // default: 20 (= 2.0x)
    pub use_jitter: bool,         // default: true
}

impl RetryPolicyConfig {
    pub const fn exponential(max_attempts: usize, base_delay: Duration) -> Self;
    pub const fn fixed(max_attempts: usize, delay: Duration) -> Self;
    pub fn delay_for_attempt(&self, attempt: usize) -> Option<Duration>;
}
```

### `ResiliencePolicy`

```rust
pub struct ResiliencePolicy {
    pub name: String,
    pub retry: Option<RetryPolicyConfig>,
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    pub bulkhead: Option<BulkheadConfig>,
    pub metadata: PolicyMetadata,
}
```

### `PolicyMetadata`

```rust
pub struct PolicyMetadata {
    pub display_name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub created_at: std::time::SystemTime,
}
```

---

## Composition

See [composition.md](composition.md) for full guide.

```rust
// LayerBuilder fluent API
let chain: ResilienceChain<String> = LayerBuilder::new()
    .with_timeout(Duration::from_secs(5))
    .with_bulkhead(max_concurrency)
    .with_circuit_breaker(breaker)
    .with_retry_exponential(3, Duration::from_millis(100))
    .build();

// Execute
let result: ResilienceResult<String> = chain.execute(|| async {
    Ok("response".to_string())
}).await;

// Execute with cancellation
let result = chain.execute_with_cancellation(
    || async { Ok("response".to_string()) },
    Some(&cancellation),
).await;
```

---

## Observability

See [observability.md](observability.md) for full guide.

### `MetricsCollector`

```rust
impl MetricsCollector {
    pub fn new(enabled: bool) -> Self;
    pub fn record(&self, name: impl Into<String>, value: f64);
    pub fn increment(&self, name: impl Into<String>);
    pub fn record_duration(&self, name: impl Into<String>, duration: Duration);
    pub fn start_timer(&self, name: impl Into<String>) -> MetricTimer;
    pub fn snapshot(&self, name: &str) -> Option<MetricSnapshot>;
    pub fn all_snapshots(&self) -> HashMap<String, MetricSnapshot>;
    pub fn reset(&self, name: &str);
    pub fn clear(&self);
}
```

### `MetricSnapshot`

```rust
pub struct MetricSnapshot {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}
```

### `MetricKind`

```rust
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}
```

### Hooks

```rust
pub trait ObservabilityHook: Send + Sync {
    fn on_event(&self, event: &PatternEvent);
}

pub struct ObservabilityHooks {
    hooks: Vec<Arc<dyn ObservabilityHook>>,
}

impl ObservabilityHooks {
    pub fn new() -> Self;
    pub fn add(&mut self, hook: Arc<dyn ObservabilityHook>);
    pub fn emit(&self, event: PatternEvent);
}
```

Built-in hooks: `LoggingHook` (log level configurable), `MetricsHook` (forwards to
`MetricsCollector`).

### Typed events

```rust
pub struct Event<C: EventCategory> {
    pub operation: String,
    pub duration: Option<Duration>,
    pub attempt: Option<usize>,
    pub max_attempts: Option<usize>,
    pub context: Option<String>,
    _marker: PhantomData<C>,
}

impl<C: EventCategory> Event<C> {
    pub fn new(operation: impl Into<String>) -> Self;
    pub fn with_duration(self, d: Duration) -> Self;
    pub fn with_attempt(self, n: usize) -> Self;
    pub fn with_max_attempts(self, n: usize) -> Self;
    pub fn with_context(self, ctx: impl Into<String>) -> Self;
}
```

Event categories:

| Type | `name()` |
|------|---------|
| `RetryEventCategory` | `"retry"` |
| `CircuitBreakerEventCategory` | `"circuit_breaker"` |
| `BulkheadEventCategory` | `"bulkhead"` |
| `TimeoutEventCategory` | `"timeout"` |
| `RateLimiterEventCategory` | `"rate_limiter"` |

---

## Prelude

```rust
use nebula_resilience::prelude::*;
```

Re-exports:

- `ResilienceError`, `ResilienceResult`
- `CircuitBreaker`, `CircuitBreakerConfig`, `CircuitState`
- `RetryStrategy`, `RetryStats`, `RetryFailure`, `JitterPolicy`
- `ExponentialBackoff`, `FixedDelay`, `LinearBackoff`
- `ConservativeCondition`, `AggressiveCondition`
- `Bulkhead`, `BulkheadConfig`
- `timeout`
- `Gate`, `GateGuard`, `GateClosed`
- `LayerBuilder`
- `ResilienceManager`, `ResiliencePolicy`
- `Retryable`
- `exponential_retry`, `fixed_retry`, `aggressive_retry`
