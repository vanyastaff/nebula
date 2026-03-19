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

### 1. `CallError<E>` — caller error type is preserved

Every pattern returns `Result<T, CallError<E>>` where `E` is the caller's own error type.
Pattern errors (`CircuitOpen`, `BulkheadFull`, `Timeout`, etc.) are separate enum variants
alongside the caller's `Operation(E)` variant. Callers never need to map into a
resilience-specific error type.

```rust
pub enum CallError<E> {
    Operation(E),           // the operation's own error
    CircuitOpen,
    BulkheadFull,
    Timeout(Duration),
    RetriesExhausted { attempts: u32, last: E },
    Cancelled { reason: Option<String> },
    LoadShed,
    RateLimited,
}
```

This design replaces the previous `ResilienceError` monolithic enum, which required
callers to convert their errors into a resilience type and back.

### 2. Plain-struct config — no const generics

Configuration is expressed as regular structs with runtime validation, not const generics.

```rust
// Old design — compile-time const generics
CircuitBreakerConfig::<5, 30_000>::new()

// Current design — plain struct, runtime validate()
CircuitBreaker::new(CircuitBreakerConfig {
    failure_threshold: 5,
    reset_timeout: Duration::from_secs(30),
    ..Default::default()
})?
```

Structs are `Serialize`/`Deserialize` — configs can be loaded from files or env at
runtime. `validate()` is called by each pattern constructor, returning `ConfigError`.

### 3. `BackoffConfig` as an enum

Backoff strategies are an enum, not a sealed-trait hierarchy. This makes configs
serialisable without needing trait objects or const-generic type parameters:

```rust
pub enum BackoffConfig {
    Fixed(Duration),
    Linear { base: Duration, max: Duration },
    Exponential { base: Duration, multiplier: f64, max: Duration },
}
```

`BackoffConfig::exponential_default()` returns the standard 100ms/2×/30s configuration.

### 4. `PipelineBuilder` / `ResiliencePipeline` composition model

`PipelineBuilder<E>` collects steps as a `Vec<Step<E>>`. `build()` validates order
and returns a `ResiliencePipeline<E>`. Execution recurses through the step list:

```
pipeline.call(f)
  │
  ├── Step::Timeout  → tokio::time::timeout wrapping remaining steps
  ├── Step::Retry    → classify_inner + retry_with loop
  ├── Step::CircuitBreaker → cb.call(remaining steps)
  └── Step::Bulkhead → bh.call(remaining steps)
        └── f()
```

`build()` emits a `tracing::warn!` if timeout appears **inside** retry (each attempt
would get its own deadline instead of a single budget across all attempts).

The recommended order is:
```
timeout → retry → circuit_breaker → bulkhead
```

Note: this differs from the legacy `LayerBuilder` which recommended
`timeout → bulkhead → circuit_breaker → retry`.

### 5. `MetricsSink` — event sink for observability

Patterns emit `ResilienceEvent` values to a `MetricsSink`. The default is `NoopSink`
(zero cost). In production, inject a custom sink that forwards to EventBus, Prometheus,
or a `MetricsCollector`. For tests, use `RecordingSink`:

```rust
pub trait MetricsSink: Send + Sync {
    fn record(&self, event: ResilienceEvent);
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
```

This replaces the `ObservabilityHooks` / `PatternEvent` system. The older `hooks` module
still exists and provides `Event<C>`, `LoggingHook`, `MetricsHook`, etc. for tracing-based
observability (not the same as `MetricsSink`).

### 6. Injectable `Clock` for deterministic testing

`CircuitBreaker` accepts a `Clock` impl for all time-based decisions. The default is
`SystemClock`. Tests can inject a mock clock to control time without `tokio::time::pause`.

### 7. `PolicySource<C>` for adaptive configuration

Any `Clone + Send + Sync` value is automatically a `PolicySource<C>` via a blanket impl:

```rust
pub trait PolicySource<C: Clone>: Send + Sync {
    fn current(&self) -> C;
}

impl<C: Clone + Send + Sync> PolicySource<C> for C { … }
```

Adaptive sources compute the config at call-time based on `LoadSignal` metrics.

### 8. `Gate` for cooperative shutdown

`Gate` / `GateGuard` is the shutdown primitive used inside `Pool<R>` (nebula-resource)
and recommended for any handler loop that needs to drain work before exiting.

Implementation uses a Tokio `Semaphore` with `u32::MAX / 2` permits. Each `enter()`
forgets one permit; `close()` acquires all `u32::MAX / 2` permits back, blocking until
every outstanding guard is dropped. An `AtomicBool` marks the gate as closing so new
`enter()` calls are rejected immediately.

---

## Module Map

```
crates/resilience/src/
│
│  ── Core types ────────────────────────────────────────────────────────────
│
├── types.rs           CallError<E> — unified error enum generic over caller error.
│                      ConfigError — returned by pattern constructors on invalid config.
│                      CallResult<T,E> = Result<T, CallError<E>>.
│
├── error.rs           ResilienceError — internal rich error (9 variants, non-exhaustive).
│                      ErrorClass (Transient/ResourceExhaustion/Permanent/Configuration/Unknown).
│                      ErrorContext — optional structured metadata.
│
├── result.rs          ResilienceResult<T> = Result<T, ResilienceError>.
│                      ResultExt — convenience methods.
│
├── cancellation.rs    CancellationContext — wraps tokio_util CancellationToken.
│                      CancellableFuture, CancellationExt.
│                      ShutdownCoordinator — multi-stage graceful drain.
│
├── policy_source.rs   PolicySource<C> trait + blanket impl for Clone types.
│
├── signals.rs         LoadSignal trait — load_factor(), error_rate(), p99_latency().
│                      ConstantLoad — test/static load signal implementation.
│
├── clock.rs           Clock trait — now() → Instant.
│                      SystemClock — production impl using std::time::Instant::now().
│
├── metrics.rs         MetricsCollector — in-process key/value metric accumulator.
│                      MetricSnapshot, MetricKind (Counter/Gauge/Histogram).
│                      Metrics — shorthand for Arc<MetricsCollector>.
│
│  ── Observability ──────────────────────────────────────────────────────────
│
├── sink.rs            MetricsSink trait — record(ResilienceEvent).
│                      NoopSink — zero-cost default.
│                      RecordingSink — records events for test assertions.
│                      ResilienceEvent — typed events emitted by patterns.
│                      CircuitState — Closed | Open | HalfOpen.
│
├── hooks.rs           ObservabilityHooks, ObservabilityHook trait, PatternEvent.
│                      Event<C: EventCategory> — typed event with builder API.
│                      EventCategory (sealed): RetryEventCategory,
│                        CircuitBreakerEventCategory, BulkheadEventCategory,
│                        TimeoutEventCategory, RateLimiterEventCategory.
│                      LoggingHook, MetricsHook — built-in hook implementations.
│                      LogLevel enum.
│
├── spans.rs           SpanGuard — RAII tracing span with success/error on drop.
│                      PatternSpanGuard<C: PatternCategory> — typed span.
│                      create_span(), record_success(), record_error().
│
│  ── Patterns ────────────────────────────────────────────────────────────────
│
├── circuit_breaker.rs CircuitBreakerConfig — plain struct, serde, validate().
│                      CircuitBreaker — Clock + MetricsSink injectable.
│                      Outcome — Success | Failure | Timeout.
│                      call() returning Result<T, CallError<E>>.
│
├── retry.rs           BackoffConfig enum — Fixed / Linear / Exponential.
│                      JitterConfig enum — None / Full { factor }.
│                      RetryConfig<E> — max_attempts, backoff, jitter, retry_if predicate.
│                      retry<F>() — free function using default exponential config.
│                      retry_with<E, F>() — free function with explicit config.
│
├── bulkhead.rs        BulkheadConfig — max_concurrency, queue_size, timeout.
│                      Bulkhead — semaphore + optional queue.
│                      call() returning Result<T, CallError<E>>.
│
├── rate_limiter.rs    RateLimiter trait — acquire(), execute(), current_rate(), reset().
│                      TokenBucket — capacity + refill rate.
│                      LeakyBucket — constant leak rate.
│                      SlidingWindow — time-window counter.
│                      AdaptiveRateLimiter — adjusts based on error rates.
│                      AnyRateLimiter — object-safe boxed wrapper.
│
├── timeout.rs         timeout_fn(duration, future) — wraps tokio::time::timeout.
│                      timeout_with_original_error() — preserves inner error type.
│                      TimeoutExecutor — struct-based alternative.
│
├── fallback.rs        FallbackStrategy<T> trait — fallback() + should_fallback().
│                      ValueFallback<T> — cloned constant value.
│                      AnyStringFallbackStrategy — string-typed erased strategy.
│
├── hedge.rs           HedgeConfig — hedge_delay, max_hedges, exponential_backoff.
│                      HedgeExecutor — FuturesUnordered parallel dispatch.
│
├── load_shed.rs       load_shed(should_shed, f) — free function predicate-based rejection.
│
├── retryable.rs       Retryable trait — is_retryable(). Blanket impls for std::io::Error.
│
│  ── Infrastructure ─────────────────────────────────────────────────────────
│
├── pipeline.rs        PipelineBuilder<E> — collects steps, validates order on build().
│                      ResiliencePipeline<E> — executes steps recursively.
│                      Step<E> — Timeout | Retry | CircuitBreaker | Bulkhead.
│
├── gate.rs            GateClosed — error when gate is already closing.
│                      GateGuard — RAII exit token; returns permit on drop.
│                      Gate — cooperative shutdown barrier (Semaphore + AtomicBool).
│
└── helpers.rs         log_result!(result, op, desc) macro.
                       print_result!(result, fmt) macro.
```

---

## Data Flow

### Pipeline execution path

```
Caller
  │  pipeline.call(|| async { ... })
  ▼
ResiliencePipeline::call
  │  run_steps(steps, idx=0, f)
  │
  ├── Step::Timeout(d)
  │     tokio::time::timeout(d, run_steps(steps, idx+1, f))
  │
  ├── Step::Retry(config)
  │     retry_with(inner_config, || classify_inner(run_steps(idx+1)))
  │     classify_inner: Ok→Ok, Operation(e)→Err(Some(e)), other→stash in bail + Err(None)
  │
  ├── Step::CircuitBreaker(cb)
  │     cb.call(|| run_inner_unwrapped(steps, idx+1, f))
  │
  └── Step::Bulkhead(bh)
        bh.call(|| run_inner_unwrapped(steps, idx+1, f))
              │
              └── idx == steps.len() → f().await.map_err(CallError::Operation)
```

### Circuit breaker execution path

```
Caller
  │  cb.call(|| async { ... })
  ▼
CircuitBreaker::call
  │  1. Load state (AtomicU8 via Clock)
  │     └─ Open + reset_timeout not elapsed → Err(CallError::CircuitOpen)
  │     └─ HalfOpen + probe_limit reached → Err(CallError::CircuitOpen)
  │  2. Execute inner future
  │  3. record_outcome(Outcome::Success | Failure | Timeout)
  │     └─ failures >= failure_threshold → transition to Open
  │     └─ any failure in HalfOpen → back to Open
  │     └─ success in HalfOpen → transition to Closed
  │     └─ sink.record(CircuitStateChanged { from, to })
  ▼
Result<T, CallError<E>>
```

### Retry execution path

```
Caller
  │  retry_with(config, || async { ... })
  ▼
retry_with
  │  loop attempt 0..max_attempts:
  │    1. Invoke factory → await future
  │    2. Ok → return Ok(value)
  │    3. Err(e):
  │       a. config.retry_if(e) == false → return Err(CallError::Operation(e))
  │       b. backoff.delay_for(attempt) + jitter → tokio::time::sleep
  │       c. sink.record(RetryAttempt { attempt, will_retry })
  │       d. attempt += 1; continue
  │    4. Budget exhausted → Err(CallError::RetriesExhausted { attempts, last })
  ▼
Result<T, CallError<E>>
```
