# nebula-resilience — Architecture

## Problem Statement

Distributed workflow actions call external services — HTTP APIs, databases, message queues.
Those services fail transiently or degrade under load. Without explicit failure handling,
a single slow dependency can saturate the async runtime, exhaust connection pools, and
cascade failures across unrelated tenants and workflows.

`nebula-resilience` centralises all failure-handling primitives behind a coherent,
type-safe API. Callers configure patterns once and reuse them across every action execution
without business logic leaking into resilience concerns.

---

## Key Design Decisions

### 1. Const generics for compile-time configuration validation

Critical numeric parameters are encoded as const generics so illegal configurations are
rejected by the Rust compiler rather than discovered at runtime.

```rust
// FAILURE_THRESHOLD = 0 is a compile error, not a runtime panic
let config = CircuitBreakerConfig::<5, 30_000>::new();
//                                  ^   ^
//                                  |   RESET_TIMEOUT_MS
//                                  FAILURE_THRESHOLD
```

Every const-generic type carries a `const VALID: ()` associated constant that is evaluated
at construction time:

```rust
const VALID: () = {
    assert!(FAILURE_THRESHOLD > 0, "FAILURE_THRESHOLD must be positive");
    assert!(RESET_TIMEOUT_MS > 0, "RESET_TIMEOUT_MS must be positive");
    assert!(RESET_TIMEOUT_MS <= 300_000, "RESET_TIMEOUT_MS must be <= 5 minutes");
};
```

This applies to `CircuitBreakerConfig`, `ExponentialBackoff`, `FixedDelay`,
`LinearBackoff`, and retry conditions.

### 2. Typestate pattern for the circuit breaker

State transitions are tracked at the type level so invalid transitions cannot be expressed:

```rust
// Phantom-type states live in core::traits::circuit_states
pub struct Closed;
pub struct Open;
pub struct HalfOpen;

pub trait TypestateCircuitState: sealed::SealedState {}
pub trait StateTransition<To: TypestateCircuitState>: TypestateCircuitState {}
```

The runtime `CircuitBreaker` uses an `AtomicU8` for the current state and a typestate
builder that ensures `with_half_open_limit` can only be called once the config is
correctly parameterised.

### 3. Sealed-trait backoff policies

`BackoffPolicy` is sealed via a private `sealed::SealedBackoff` supertrait. External crates
cannot add new backoff implementations unless they go through the public `CustomBackoff`
escape hatch. This keeps the matching exhaustive inside the crate:

```rust
mod sealed {
    pub trait SealedBackoff {}
}

pub trait BackoffPolicy: sealed::SealedBackoff + Send + Sync + … {
    fn calculate_delay(&self, attempt: usize) -> Duration;
    fn max_delay(&self) -> Duration;
    fn policy_name(&self) -> &'static str;
}
```

The same pattern applies to `EventCategory` (observability), `PatternCategory`, and
`ServiceCategory` (manager).

### 4. Layer composition model

Individual patterns (`timeout`, `Bulkhead`, `CircuitBreaker`, retry) are wrapped as
`ResilienceLayer<T>` middleware. A `LayerStack<T>` chains layers and dispatches to the
innermost operation. `LayerBuilder` provides the fluent API without exposing the
internal stack structure to callers.

The composition order enforced by `LayerBuilder` matches the recommended production order:

```
timeout → bulkhead → circuit_breaker → retry → (operation)
```

Each layer receives a `BoxedOperation<T>` wrapping the original future factory and a
reference to the remaining `LayerStack<T>`. The stack is immutable once built.

### 5. Error classification for uniform handling

Every `ResilienceError` variant implements `classify() → ErrorClass`:

| `ErrorClass` | Variants | `is_retryable()` |
|---|---|---|
| `Transient` | `Timeout`, `Custom { retryable: true }` | `true` |
| `ResourceExhaustion` | `CircuitBreakerOpen`, `BulkheadFull`, `RateLimitExceeded` | `true` |
| `Permanent` | `RetryLimitExceeded`, `FallbackFailed`, `Cancelled` | `false` |
| `Configuration` | `InvalidConfig` | `false` |
| `Unknown` | catch-all | `false` |

This lets upstream code handle errors uniformly without pattern-matching every variant:

```rust
if error.is_retryable() {
    // schedule for re-queue
} else if error.is_terminal() {
    // mark node as failed
}
```

### 6. `Gate` for cooperative shutdown

`Gate` / `GateGuard` is the shutdown primitive used inside `Pool<R>` (nebula-resource)
and recommended for any handler loop that needs to drain work before exiting.

The implementation uses a Tokio `Semaphore` with `u32::MAX / 2` permits. Each
`enter()` forgets one permit; `close()` acquires all `u32::MAX / 2` permits back,
blocking until every outstanding guard is dropped. An `AtomicBool` marks the gate as
closing so new `enter()` calls are rejected immediately.

### 7. `ResilienceManager` as a typed service registry

Services are identified by a compile-time `Service` trait constant:

```rust
pub trait Service: Send + Sync + 'static {
    const NAME: &'static str;
    type Category: ServiceCategory;
}
```

The manager stores polices in a `DashMap<String, ServicePolicy>` keyed by `Service::NAME`.
Execution paths:
- `execute_typed::<S>(op)` — fully typed; `S::NAME` is resolved at compile time.
- `execute(name, op)` — dynamic string name; used by the policy engine.

---

## Module Map

```
crates/resilience/src/
│
│  ── Core ─────────────────────────────────────────────────────────────────
│
├── core/error.rs          ResilienceError — non-exhaustive enum, 9 variants.
│                          ErrorClass (Transient / ResourceExhaustion / Permanent /
│                            Configuration / Unknown).
│                          ErrorContext — optional structured metadata.
│                          classify(), is_retryable(), is_terminal(), retry_after().
│
├── core/result.rs         ResilienceResult<T> = Result<T, ResilienceError>.
│                          ResultExt — convenience methods on Result.
│
├── core/traits.rs         Executable — single async execute() method.
│                          HealthCheck — async health_check() → HealthStatus.
│                          PatternMetrics — pattern-level counter/histogram.
│                          Retryable — is_retryable() for error types.
│                          circuit_states::{Closed, Open, HalfOpen,
│                            TypestateCircuitState, StateTransition}.
│
├── core/config.rs         Configurable, ConfigBuilder, ConfigSource.
│                          ResilienceConfig — runtime configuration value.
│                          ResilienceConfigManager — hot-reload bridge to nebula-config.
│                          NebulaConfig, CommonConfig.
│
├── core/types.rs          Newtypes: RetryCount, RateLimit, Timeout, MaxConcurrency,
│                            FailureThreshold, DurationExt, ResilienceResultExt.
│
├── core/cancellation.rs   CancellationContext — wraps tokio_util CancellationToken.
│                          CancellableFuture, CancellationExt.
│                          ShutdownCoordinator — multi-stage graceful drain.
│
├── core/advanced.rs       Typestate PolicyBuilder: Unconfigured → WithRetry →
│                            WithCircuitBreaker → Complete.
│                          ComposedPolicy — combined retry + circuit-breaker config.
│                          StrategyConfig, Strategy (Conservative / Balanced / Aggressive).
│                          ValidatedRetryConfig — const-validated wrapper.
│
├── core/categories.rs     Sealed category system.
│                          PatternCategory: Retry | Timeout | Protection |
│                            FlowControl | Fallback.
│                          ServiceCategory: Http | Database | MessageQueue |
│                            Cache | Generic.
│                          Category — unified marker trait.
│
├── core/metrics.rs        MetricsCollector — in-process metrics with security guards
│                            (max 10 000 keys, max 256-char names, NaN rejection).
│                          Metric — per-key accumulator.
│                          MetricSnapshot — read-only point-in-time snapshot.
│                          MetricKind: Counter | Gauge | Histogram.
│                          MetricTimer — RAII duration recorder.
│
├── core/dynamic/          DynamicConfig, DynamicConfigBuilder, DynamicConfigurable.
│                          ResiliencePresets — named preset configurations.
│                          Per-pattern builders: RetryConfigBuilder,
│                            CircuitBreakerConfigBuilder, BulkheadConfigBuilder.
│
│  ── Patterns ──────────────────────────────────────────────────────────────
│
├── patterns/circuit_breaker.rs
│                          CircuitBreakerConfig<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>.
│                          CircuitBreaker<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>.
│                          State enum: Closed | Open | HalfOpen.
│                          CircuitBreakerStats — failure count, state, last transition.
│                          Preset type aliases: StandardCircuitBreaker, FastCircuitBreaker,
│                            SlowCircuitBreaker.
│                          Preset constructors: standard_config(), fast_config(), slow_config().
│
├── patterns/retry.rs      BackoffPolicy (sealed): FixedDelay<DELAY_MS>,
│                            ExponentialBackoff<BASE_MS, MULTIPLIER_X10, MAX_MS>,
│                            LinearBackoff<STEP_MS, MAX_MS>, CustomBackoff.
│                          RetryCondition (sealed): ConservativeCondition<N>,
│                            AggressiveCondition<N>, TimeBasedCondition<MAX_MS>.
│                          JitterPolicy: None | Full | Equal.
│                          RetryConfig<B, C> — combines backoff and condition.
│                          RetryStrategy<B, C, MAX_ATTEMPTS> — executes with telemetry.
│                          RetryStats — attempts, total_duration, delays.
│                          RetryFailure<E> — wraps error + stats on exhaustion.
│                          Convenience constructors: exponential_retry::<N>(),
│                            fixed_retry::<DELAY_MS, N>(), aggressive_retry::<N>().
│
├── patterns/bulkhead.rs   BulkheadConfig — max_concurrency, queue_size, timeout.
│                          Bulkhead — semaphore + AtomicUsize waiting_count.
│                          BulkheadStats — active, waiting, available.
│                          execute(), try_execute() — async operations.
│
├── patterns/rate_limiter/ RateLimiter — token-bucket implementation.
│                          RateLimiterConfig — rate, burst, window.
│                          execute(), try_execute() — rate-checked dispatch.
│
├── patterns/timeout.rs    timeout(duration, future) — wraps tokio::time::timeout,
│                            maps Elapsed to ResilienceError::Timeout.
│
├── patterns/fallback.rs   FallbackStrategy<T> trait — fallback() + should_fallback().
│                          ValueFallback<T> — returns a cloned constant.
│                          FunctionFallback<T, F, Fut> — closure-based fallback.
│                          CacheFallback<T> — TTL-controlled cached last-good value.
│                          ChainedFallback<T> — ordered list of strategies.
│                          execute_with_fallback(primary, strategy) — top-level helper.
│
├── patterns/hedge.rs      HedgeConfig — hedge_delay, max_hedges, exponential_backoff.
│                          HedgeExecutor — FuturesUnordered-based parallel issue.
│                          AdaptiveHedgeExecutor — dynamically adjusts hedge delay
│                            based on observed latency distribution.
│
│  ── Infrastructure ────────────────────────────────────────────────────────
│
├── compose.rs             BoxedOperation<T> — type-erased operation wrapper.
│                          ResilienceLayer<T> trait — apply() + name().
│                          LayerStack<T> trait — execute() + execute_with_cancellation().
│                          ResilienceChain<T> — ordered Vec of dyn ResilienceLayer.
│                          LayerBuilder — fluent construction with_timeout(),
│                            with_retry_exponential(), with_circuit_breaker(),
│                            with_bulkhead(), build().
│
├── gate.rs                GateClosed — error returned when gate is already closing.
│                          GateGuard — RAII exit token; adds back one semaphore permit.
│                          Gate — cooperative shutdown barrier backed by Semaphore +
│                            AtomicBool. enter() / close().await.
│
├── manager.rs             Service trait — NAME const + Category associated type.
│                          Operation trait — NAME const + IDEMPOTENT flag.
│                          RetryableOperation<T> trait — object-safe execute().
│                          ServicePolicy — combined circuit breaker + policy config.
│                          ResilienceManager — DashMap registry + AtomicU64 counters.
│                          execute_typed::<S>(), execute(), execute_with_cancellation().
│
├── policy.rs              RetryPolicyConfig — serialisable retry params.
│                          ResiliencePolicy — named policy with metadata.
│                          PolicyMetadata — display name, description, tags.
│
├── retryable.rs           Blanket Retryable impls for std::io::Error, reqwest, sqlx.
│
├── helpers.rs             log_result!(result, op, desc) macro.
│                          print_result!(result, fmt) macro.
│
│  ── Observability ─────────────────────────────────────────────────────────
│
├── observability/hooks.rs EventCategory (sealed): RetryEventCategory,
│                            CircuitBreakerEventCategory, BulkheadEventCategory,
│                            TimeoutEventCategory, RateLimiterEventCategory.
│                          Event<C: EventCategory> — typed event with duration,
│                            attempt count, max_attempts, context string.
│                          Metric — label-keyed f64 accumulator.
│                          metrics module — operation_histogram(), state_gauge(),
│                            error_counter() helper constructors.
│                          ObservabilityHook trait — on_event() callback.
│                          ObservabilityHooks — ordered Vec<Arc<dyn ObservabilityHook>>.
│                          LogLevel, LoggingHook, MetricsHook — built-in impls.
│                          PatternEvent — untyped event for legacy paths.
│
└── observability/spans.rs SpanGuard — RAII tracing span that records success/error
│                            on drop.
│                          PatternCategory, PatternSpanGuard — pattern-classified span.
│                          create_span(name, category), record_success(),
│                            record_error().
```

---

## Data Flow

### Circuit breaker execution path

```
Caller
  │  breaker.execute(|| async { ... })
  ▼
CircuitBreaker::execute
  │  1. Load AtomicU8 state
  │     └─ Open → check reset window → still open → Error::CircuitBreakerOpen
  │     └─ HalfOpen → check probe limit → limit reached → Error::CircuitBreakerOpen
  │  2. Execute inner future
  │  3. On success → record success, maybe transition HalfOpen → Closed
  │  4. On failure → record failure
  │     └─ failure_count >= FAILURE_THRESHOLD → transition Closed → Open
  │     └─ any failure in HalfOpen → transition → Open
  ▼
CallerResult
```

### Retry execution path

```
Caller
  │  strategy.execute(|| async { ... })
  ▼
RetryStrategy::execute
  │  loop attempt 0..MAX_ATTEMPTS:
  │    1. Invoke factory → await future
  │    2. Success → return (value, RetryStats)
  │    3. Error:
  │       a. condition.should_retry(attempt, &error) → false → return RetryFailure
  │       b. backoff.calculate_delay(attempt) + jitter → sleep
  │       c. attempt += 1; continue
  │    4. Budget exhausted → RetryFailure { error, stats }
  ▼
Result<(T, RetryStats), RetryFailure<E>>
```

### Layer composition execution path

```
Caller
  │  chain.execute(|| async { ... })
  ▼
ResilienceChain::execute
  │  BoxedOperation wraps caller's factory
  │  Calls layers[0].apply(op, remaining_stack)
  │
  ├── TimeoutLayer::apply
  │     tokio::select! on op vs deadline
  │
  ├── BulkheadLayer::apply
  │     acquire semaphore permit → execute → release
  │
  ├── CircuitBreakerLayer::apply
  │     check state → execute → record outcome
  │
  └── RetryLayer::apply
        loop with backoff → execute → return
        │
        └─ innermost: BoxedOperation::execute() → caller's future
```
