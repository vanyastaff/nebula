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
- [Load Shed](#load-shed)
- [Gate](#gate)
- [Pipeline](#pipeline)
- [Sink / Events](#sink--events)
- [Observability Hooks](#observability-hooks)
- [Signals](#signals)
- [Policy Source](#policy-source)
- [Cancellation](#cancellation)
- [Prelude / Re-exports](#prelude--re-exports)

---

## Core Types

### `CallError<E>`

Returned by all resilience operations. `E` is the caller's own error type.

```rust
pub enum CallError<E> {
    /// The operation returned an error (possibly after retries).
    Operation(E),
    /// Circuit breaker is open — request rejected immediately.
    CircuitOpen,
    /// Bulkhead is at capacity — request rejected.
    BulkheadFull,
    /// Timeout elapsed before the operation completed.
    Timeout(Duration),
    /// All retry attempts exhausted.
    RetriesExhausted { attempts: u32, last: E },
    /// Operation was cancelled.
    Cancelled { reason: Option<String> },
    /// Load shed — system is overloaded, request rejected.
    LoadShed,
    /// Rate limit exceeded.
    RateLimited,
}

impl<E> CallError<E> {
    /// True only if this is a `Cancelled` variant.
    pub const fn is_cancellation(&self) -> bool;

    /// All pattern errors return false — operation retryability is predicate-driven.
    pub const fn is_retriable(&self) -> bool;

    /// Map the inner operation error, leaving pattern errors unchanged.
    pub fn map_operation<F, E2>(self, f: F) -> CallError<E2>
    where F: FnOnce(E) -> E2;
}
```

---

### `ConfigError`

Returned by pattern constructors when configuration is invalid.

```rust
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid resilience config: {message}")]
pub struct ConfigError {
    pub field: &'static str,
    pub message: String,
}

impl ConfigError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self;
}
```

---

### `CallResult<T, E>`

```rust
pub type CallResult<T, E> = Result<T, CallError<E>>;
```

---

## Circuit Breaker

### `CircuitBreakerConfig`

Plain struct, `Serialize` / `Deserialize`, `Default`.

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,         // Min: 1. Default: 5
    pub reset_timeout: Duration,         // Default: 30s
    pub half_open_max_ops: u32,          // Default: 1
    pub min_operations: u32,             // Default: 5
    pub failure_rate_threshold: f64,     // 0.0..=1.0, validated but not yet used. Default: 0.5
    pub sliding_window: Duration,        // Default: 60s
    pub count_timeouts_as_failures: bool,// Default: true
}

impl CircuitBreakerConfig {
    /// Validate. Called internally by CircuitBreaker::new().
    pub fn validate(&self) -> Result<(), ConfigError>;
}
```

---

### `CircuitBreaker`

```rust
impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Result<Self, ConfigError>;

    /// New with custom sink for observability.
    pub fn with_sink(config: CircuitBreakerConfig, sink: Arc<dyn MetricsSink>)
        -> Result<Self, ConfigError>;

    /// Execute the operation through the circuit breaker.
    pub async fn call<T, E, F, Fut>(&self, f: F) -> Result<T, CallError<E>>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send;

    /// Manually record an outcome (for use with non-closure-based callers).
    pub fn record_outcome(&self, outcome: Outcome);

    /// Current circuit state.
    pub fn state(&self) -> CircuitState;
}
```

### `Outcome`

```rust
#[derive(Debug, Clone, Copy)]
pub enum Outcome {
    Success,
    Failure,
    Timeout,
}
```

---

## Retry

### `BackoffConfig`

```rust
#[derive(Debug, Clone)]
pub enum BackoffConfig {
    Fixed(Duration),
    Linear { base: Duration, max: Duration },
    Exponential { base: Duration, multiplier: f64, max: Duration },
}

impl BackoffConfig {
    /// Standard exponential: 100ms base, 2× multiplier, 30s cap.
    pub const fn exponential_default() -> Self;
}
```

---

### `JitterConfig`

```rust
#[derive(Debug, Clone, Default)]
pub enum JitterConfig {
    #[default]
    None,
    /// Add a random fraction up to `factor` (0.0–1.0) of the delay.
    Full { factor: f64 },
}
```

---

### `RetryConfig<E>`

```rust
impl<E: Send + 'static> RetryConfig<E> {
    /// Create with `max_attempts`. Returns `Err(ConfigError)` if 0.
    pub fn new(max_attempts: u32) -> Result<Self, ConfigError>;

    /// Set backoff strategy.
    pub fn backoff(self, config: BackoffConfig) -> Self;

    /// Set jitter.
    pub fn jitter(self, config: JitterConfig) -> Self;

    /// Set a predicate controlling which errors are retried.
    /// By default all errors are retried.
    pub fn retry_if(self, predicate: impl Fn(&E) -> bool + Send + Sync + 'static) -> Self;
}
```

---

### Free functions

```rust
/// Retry `f` up to 3 times with exponential_default() backoff, retrying all errors.
pub async fn retry<T, E, F, Fut>(f: F) -> Result<T, CallError<E>>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Send + 'static;

/// Retry `f` using explicit `config`.
pub async fn retry_with<T, E, F, Fut>(
    config: RetryConfig<E>,
    f: F,
) -> Result<T, CallError<E>>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send,
    T: Send + 'static,
    E: Send + 'static;
```

---

## Bulkhead

### `BulkheadConfig`

```rust
pub struct BulkheadConfig {
    pub max_concurrency: usize,     // Default: 10
    pub queue_size: usize,          // Default: 100
    pub timeout: Option<Duration>,  // Default: Some(30s)
}
```

---

### `Bulkhead`

```rust
impl Bulkhead {
    pub fn new(max_concurrency: usize) -> Self;
    pub fn with_config(config: BulkheadConfig) -> Self;

    pub async fn call<T, E, F, Fut>(&self, f: F) -> Result<T, CallError<E>>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send;

    pub fn active_operations(&self) -> usize;
    pub fn available_permits(&self) -> usize;
    pub fn max_concurrency(&self) -> usize;
    pub fn is_at_capacity(&self) -> bool;
}
```

---

## Rate Limiter

### `RateLimiter` trait

```rust
#[allow(async_fn_in_trait)]
pub trait RateLimiter: Send + Sync {
    async fn acquire(&self) -> Result<(), CallError<()>>;

    async fn execute<T, E, F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<T, E>> + Send,
        T: Send;

    async fn current_rate(&self) -> f64;
    async fn reset(&self);
}
```

### Implementations

| Type | Algorithm | Constructor |
|------|-----------|-------------|
| `TokenBucket` | Token bucket | `TokenBucket::new(capacity: usize, refill_rate: f64)` |
| `LeakyBucket` | Leaky bucket | `LeakyBucket::new(capacity: usize, leak_rate: f64)` |
| `SlidingWindow` | Sliding time window | `SlidingWindow::new(max_requests: usize, window: Duration)` |
| `AdaptiveRateLimiter` | Error-rate adaptive | `AdaptiveRateLimiter::new(base_rate: f64)` |

### `AnyRateLimiter`

Object-safe boxed rate limiter:

```rust
pub struct AnyRateLimiter(Arc<dyn RateLimiter>);

impl AnyRateLimiter {
    pub fn new(limiter: impl RateLimiter + 'static) -> Self;
}
```

---

## Timeout

```rust
/// Hard deadline. Returns Err(CallError::Timeout) on expiry.
pub async fn timeout_fn<T, E, Fut>(
    duration: Duration,
    future: Fut,
) -> Result<T, CallError<E>>
where
    Fut: Future<Output = Result<T, E>>;

/// Same but wraps timeout error in the original error type via From.
pub async fn timeout_with_original_error<T, E, Fut>(
    duration: Duration,
    future: Fut,
) -> Result<T, E>
where
    Fut: Future<Output = Result<T, E>>,
    E: From<CallError<E>>;

pub struct TimeoutExecutor {
    pub duration: Duration,
}

impl TimeoutExecutor {
    pub fn new(duration: Duration) -> Self;
    pub async fn call<T, E, F, Fut>(&self, f: F) -> Result<T, CallError<E>>
    where …;
}
```

---

## Fallback

### `FallbackStrategy<T>`

```rust
#[async_trait]
pub trait FallbackStrategy<T>: Send + Sync {
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<T>;
    fn should_fallback(&self, error: &ResilienceError) -> bool { true }
}
```

### `ValueFallback<T>`

Returns a cloned constant value:

```rust
let fallback = ValueFallback::new("default".to_string());
```

### `AnyStringFallbackStrategy`

Type-erased fallback over `String` errors.

---

## Hedge

### `HedgeConfig`

```rust
pub struct HedgeConfig {
    pub hedge_delay: Duration,       // Default: 50ms
    pub max_hedges: usize,           // Default: 2
    pub exponential_backoff: bool,   // Default: true
    pub backoff_multiplier: f64,     // Default: 2.0
}
```

### `HedgeExecutor`

```rust
impl HedgeExecutor {
    pub fn new(config: HedgeConfig) -> Self;

    pub async fn execute<T, F, Fut>(&self, operation: F) -> Result<T, CallError<()>>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = Result<T, ()>> + Send,
        T: Send;
}
```

---

## Load Shed

```rust
/// Reject immediately when `should_shed()` returns true.
///
/// Returns Err(CallError::LoadShed) if shed, otherwise executes f().
pub async fn load_shed<T, E, S, F>(
    should_shed: S,
    f: F,
) -> Result<T, CallError<E>>
where
    S: Fn() -> bool,
    F: FnOnce() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>;
```

Integrate with `LoadSignal` for adaptive decisions:

```rust
let signal: Arc<dyn LoadSignal> = …;
let result = load_shed(
    || signal.load_factor() > 0.9,
    || Box::pin(do_work()),
).await;
```

---

## Gate

See [gate.md](gate.md) for full usage guide.

```rust
pub struct Gate;  // Clone — all clones share the same underlying state

impl Gate {
    pub fn new() -> Self;
    pub fn enter(&self) -> Result<GateGuard, GateClosed>;
    pub async fn close(&self);
    pub fn is_closed(&self) -> bool;
}

pub struct GateGuard;  // RAII; returns one permit on drop

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("gate is closed — new enter() calls are rejected")]
pub struct GateClosed;
```

---

## Pipeline

See [composition.md](composition.md) for full guide.

```rust
pub struct PipelineBuilder<E: 'static> { … }

impl<E: Send + 'static> PipelineBuilder<E> {
    pub const fn new() -> Self;

    #[must_use] pub fn timeout(self, d: Duration) -> Self;
    #[must_use] pub fn retry(self, config: RetryConfig<E>) -> Self;
    #[must_use] pub fn circuit_breaker(self, cb: Arc<CircuitBreaker>) -> Self;
    #[must_use] pub fn bulkhead(self, bh: Arc<Bulkhead>) -> Self;

    /// Emits tracing::warn! if timeout is inside retry.
    #[must_use] pub fn build(self) -> ResiliencePipeline<E>;
}

pub struct ResiliencePipeline<E: 'static> { … }

impl<E: Send + 'static> ResiliencePipeline<E> {
    pub const fn builder() -> PipelineBuilder<E>;

    pub async fn call<T, F>(&self, f: F) -> Result<T, CallError<E>>
    where
        T: Send + 'static,
        F: Fn() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>
            + Clone + Send + Sync + 'static;
}
```

---

## Sink / Events

See [observability.md](observability.md) for full guide.

```rust
pub trait MetricsSink: Send + Sync {
    fn record(&self, event: ResilienceEvent);
}

pub struct NoopSink;    // default, zero cost
pub struct RecordingSink;

impl RecordingSink {
    pub fn new() -> Self;
    pub fn events(&self) -> Vec<ResilienceEvent>;
    pub fn count(&self, kind: &str) -> usize;
    pub fn has_state_change(&self, to: CircuitState) -> bool;
}

pub enum ResilienceEvent {
    CircuitStateChanged { from: CircuitState, to: CircuitState },
    RetryAttempt { attempt: u32, will_retry: bool },
    BulkheadRejected,
    TimeoutElapsed { duration: Duration },
    HedgeFired { hedge_number: u32 },
    RateLimitExceeded,
    LoadShed,
}

pub enum CircuitState { Closed, Open, HalfOpen }
```

---

## Observability Hooks

Legacy hook system for `tracing`-based observability. Separate from `MetricsSink`.

```rust
pub trait ObservabilityHook: Send + Sync {
    fn on_event(&self, event: &PatternEvent);
}

pub struct ObservabilityHooks { … }

impl ObservabilityHooks {
    pub fn new() -> Self;
    pub fn add(&mut self, hook: Arc<dyn ObservabilityHook>);
    pub fn emit(&self, event: PatternEvent);
    pub fn hook_count(&self) -> usize;
}

pub struct PatternEvent {
    pub pattern: String,
    pub operation: String,
    pub duration: Option<Duration>,
    pub success: bool,
    pub error: Option<String>,
    pub metadata: HashMap<String, String>,
}

/// Typed event with compile-time category.
pub struct Event<C: EventCategory> { … }

impl<C: EventCategory> Event<C> {
    pub fn new(operation: impl Into<String>) -> Self;
    pub fn with_duration(self, d: Duration) -> Self;
    pub fn with_error(self, error: impl Into<String>) -> Self;
    pub fn with_context(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn category(&self) -> &'static str;
    pub fn is_error(&self) -> bool;
}
```

Event category markers (sealed):

| Type | `name()` | Default log level |
|------|---------|------------------|
| `RetryEventCategory` | `"retry"` | `Info` |
| `CircuitBreakerEventCategory` | `"circuit_breaker"` | `Warn` |
| `BulkheadEventCategory` | `"bulkhead"` | `Info` |
| `TimeoutEventCategory` | `"timeout"` | `Warn` |
| `RateLimiterEventCategory` | `"rate_limiter"` | `Info` |

Built-in hooks: `LoggingHook` (configurable `LogLevel`), `MetricsHook` (forwards to `MetricsCollector`).

---

## Signals

```rust
pub trait LoadSignal: Send + Sync {
    fn load_factor(&self) -> f64;   // 0.0 idle .. 1.0 saturated
    fn error_rate(&self) -> f64;    // 0.0..1.0
    fn p99_latency(&self) -> Duration;
}

pub struct ConstantLoad {
    pub factor: f64,
    pub error_rate: f64,
    pub p99_latency: Duration,
}

impl ConstantLoad {
    pub const fn idle() -> Self;      // 0% load, 0% errors, 5ms latency
    pub const fn saturated() -> Self; // 100% load, 50% errors, 2s latency
}
```

---

## Policy Source

```rust
pub trait PolicySource<C: Clone>: Send + Sync {
    fn current(&self) -> C;
}

// Blanket impl — any Clone + Send + Sync value is a static PolicySource.
impl<C: Clone + Send + Sync> PolicySource<C> for C { … }
```

---

## Cancellation

```rust
pub struct CancellationContext { … }

impl CancellationContext {
    pub fn from_token(token: tokio_util::sync::CancellationToken) -> Self;
    pub fn is_cancelled(&self) -> bool;
}

pub struct ShutdownCoordinator { … }

impl ShutdownCoordinator {
    pub fn new() -> Self;
    pub fn token(&self) -> CancellationContext;
    pub fn shutdown(&self);
    pub async fn wait(&self);
}
```

---

## Prelude / Re-exports

`use nebula_resilience::*;` provides:

- `CallError<E>`, `CallResult<T, E>`, `ConfigError`
- `ResilienceError`, `ErrorClass`, `ErrorContext`
- `ResilienceResult<T>`, `ResultExt`
- `CancellationContext`, `CancellableFuture`, `CancellationExt`, `ShutdownCoordinator`
- `PolicySource`
- `LoadSignal`, `ConstantLoad`
- `MetricKind`, `MetricSnapshot`, `MetricsCollector`, `Metrics`
- `Bulkhead`, `BulkheadConfig`
- `CircuitBreaker`, `CircuitBreakerConfig`, `Outcome`
- `FallbackStrategy`, `ValueFallback`, `AnyStringFallbackStrategy`
- `HedgeConfig`, `HedgeExecutor`
- `load_shed`
- `AdaptiveRateLimiter`, `AnyRateLimiter`, `LeakyBucket`, `RateLimiter`, `SlidingWindow`, `TokenBucket`
- `BackoffConfig`, `JitterConfig`, `RetryConfig`, `retry`, `retry_with`
- `TimeoutExecutor`, `timeout_fn`, `timeout_with_original_error`
- `BulkheadEventCategory`, `CircuitBreakerEventCategory`, `Event`, `EventCategory`
- `LogLevel`, `LoggingHook`, `Metric`, `MetricsHook`, `ObservabilityHook`, `ObservabilityHooks`
- `PatternEvent`, `RateLimiterEventCategory`, `RetryEventCategory`, `TimeoutEventCategory`
- `CircuitState`, `MetricsSink`, `NoopSink`, `RecordingSink`, `ResilienceEvent`
- `PatternCategory`, `PatternSpanGuard`, `SpanGuard`, `create_span`, `record_error`, `record_success`
- `Gate`, `GateClosed`, `GateGuard`
- `PipelineBuilder`, `ResiliencePipeline`
