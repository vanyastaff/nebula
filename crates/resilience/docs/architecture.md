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
    RateLimited { retry_after: Option<Duration> },
    FallbackFailed { reason: Option<String> },
    FallbackFailedWithContext { primary, fallback },
}
```

This design replaces the previous `ResilienceError` monolithic enum, which required
callers to convert their errors into a resilience type and back.
`FallbackFailedWithContext` is used where the crate can preserve both sides of a
failed degradation path, for example `FunctionFallback` failures. The generic
`FallbackStrategy::fallback(error)` method is a safe wrapper that checks
`should_fallback()` before recovery, so cancellation and overload-style policy
rejections are not recovered by built-in strategies, chains, or priority dispatch
unless a custom strategy explicitly opts in. Strategy recovery still receives the
primary error by value, so universal primary-error preservation for custom fallback
failures remains a future trait-level correction.

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

`PipelineBuilder<E>` collects steps as a `Vec<Step<E>>`. `build()` keeps compatibility
and warns on dangerous order, `build_checked()` rejects order inversions with
`ConfigError`, and `build_recommended_order()` sorts config-driven policies into the
safe order. Execution recurses through the step list:

```text
pipeline.call(f)
  │
  ├── Step::LoadShed → reject before expensive policy work
  ├── Step::RateLimiter → reject once before retry unless explicitly ordered inside retry
  ├── Step::Timeout  → tokio::time::timeout wrapping remaining steps
  ├── Step::Retry    → classifier-aware retry_with loop
  ├── Step::CircuitBreaker → cb.call(remaining steps)
  └── Step::Bulkhead → bh.call(remaining steps)
        └── f()
```

`build()` emits a `tracing::warn!` if timeout appears **inside** retry (each attempt
would get its own deadline instead of a single budget across all attempts).
`build_checked()` returns a config error for any deviation from the recommended order.

The recommended order is:

```text
load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead
```

Note: this differs from the legacy `LayerBuilder` which recommended
`timeout → bulkhead → circuit_breaker → retry`.

Operation errors are permanent by default in a pipeline retry. Callers must opt into
replay with `RetryConfig::retry_if`, `RetryConfig::with_classifier`,
`PipelineBuilder::classifier`, or `PipelineBuilder::classify_errors()`. This avoids
accidentally retrying non-idempotent workflow actions.

`ResiliencePipeline::call_with_context()` threads cooperative cancellation through
the operation and major policy waits. `call_with_context_and_fallback()` extends
that contract to fallback execution: cancellation wins before fallback starts and
while the fallback future is running, preventing shutdown from being reported as a
successful fallback recovery.

`PolicyContext` is the forward-compatible runtime context for Nebula engine
integration. It groups cancellation, a total monotonic deadline, and
low-cardinality `PolicyScope`. `call_with_policy_context()` and
`call_with_policy_context_and_fallback()` apply the deadline to the whole call and
use context scope for `PipelineCompleted` when provided.
Standalone `Bulkhead`, `RateLimiter`, `CircuitBreaker`, and `FallbackOperation`
also expose context-aware entry points so callers that do not use a pipeline still
preserve the same cancellation/deadline contract.

### 5. `MetricsSink` — event sink for observability

Patterns emit `ResilienceEvent` values to a `MetricsSink`. The default is `NoopSink`
(zero cost). In production, inject a custom sink that forwards to EventBus, Prometheus,
or your metrics backend. For tests, use `RecordingSink`:

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
    FallbackAttempted { primary_error: CallErrorKind },
    FallbackSucceeded { primary_error: CallErrorKind },
    FallbackFailed {
        primary_error: CallErrorKind,
        fallback_error: CallErrorKind,
    },
    PipelineCompleted { scope: PolicyScope, outcome: PipelineOutcome },
}
```

`ResilienceEvent::kind()` returns a `ResilienceEventKind` enum for counting/filtering.
`RecordingSink::count()` takes a `ResilienceEventKind` variant (not a string).
`PipelineCompleted` is the high-level event that distinguishes primary success,
primary failure, fallback success, and fallback failure for a caller-provided scope.

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

`close_with_timeout()` is the bounded shutdown variant for runtime owners that cannot
wait forever. It leaves the gate closing, returns `GateCloseTimeout`, and reports a
best-effort active guard count for diagnostics.

---

## Module Map

```text
crates/resilience/src/
│
│  ── Core types ────────────────────────────────────────────────────────────
│
├── error.rs           CallError<E> — unified error enum generic over caller error.
│                      CallErrorKind — discriminant enum for pattern matching.
│                      ConfigError — returned by pattern constructors on invalid config.
│                      CallResult<T,E> = Result<T, CallError<E>>.
│
├── cancellation.rs    CancellationContext — wraps tokio_util CancellationToken.
│                      CancellableFuture, CancellationExt.
│
├── context.rs         PolicyContext — cancellation + deadline + scope.
│
├── deadline.rs        Deadline — monotonic start + budget helper used for
│                      remaining-budget enforcement.
│
├── policy.rs          PolicySource<C> trait + blanket impl for Clone types.
│                      LoadSignal trait — load_factor(), error_rate(), p99_latency().
│                      ConstantLoad — test/static load signal implementation.
│
├── clock.rs           Clock trait — now() → Instant.
│                      SystemClock — production impl using std::time::Instant::now().
│
│  ── Observability ──────────────────────────────────────────────────────────
│
├── sink.rs            MetricsSink trait — record(ResilienceEvent).
│                      NoopSink — zero-cost default.
│                      RecordingSink — records events for test assertions.
│                      ResilienceEvent — typed events emitted by patterns.
│                      ResilienceEventKind — discriminant enum for counting/filtering.
│                      CircuitState — Closed | Open | HalfOpen.
│                      PolicyScope, ScopeValue, PipelineOutcome.
│
│  ── Patterns ────────────────────────────────────────────────────────────────
│
├── circuit_breaker.rs CircuitBreakerConfig — plain struct, serde, validate().
│                      CircuitBreaker — Clock + MetricsSink injectable.
│                      Outcome — Success | Failure | Timeout.
│                      call() and context-aware call methods.
│
├── retry.rs           BackoffConfig enum — Fixed / Linear / Exponential.
│                      JitterConfig enum — None / Full { factor }.
│                      RetryConfig<E> — max_attempts, backoff, jitter, retry_if predicate.
│                      retry<F>() — free function using default exponential config.
│                      retry_with<E, F>() — free function with explicit config.
│
├── bulkhead.rs        BulkheadConfig — max_concurrency, queue_size, timeout.
│                      Bulkhead — semaphore + optional queue.
│                      call()/acquire() plus context-aware variants.
│
├── rate_limiter.rs    RateLimiter trait — acquire(), call() (default impl),
│                      context-aware acquire/call.
│                      ErasedRateLimiter — object-safe facade for registries.
│                      TokenBucket — capacity + refill rate.
│                      LeakyBucket — constant leak rate.
│                      SlidingWindow — time-window counter.
│                      AdaptiveRateLimiter — adjusts based on error rates.
│
├── timeout.rs         timeout(duration, future) — wraps tokio::time::timeout.
│                      TimeoutExecutor — struct-based alternative.
│
├── fallback.rs        FallbackStrategy<T, E> trait — safe fallback() + recover().
│                      ValueFallback<T> — cloned constant value.
│                      ChainFallback<T, E> — chains multiple fallbacks via then().
│
├── hedge.rs           HedgeConfig — hedge_delay, max_hedges, exponential_backoff,
│                      duplicate_safety.
│                      HedgeSafety — requires Idempotent before duplicate execution.
│                      HedgeExecutor — JoinSet parallel dispatch.
│                      AdaptiveHedgeExecutor — adaptive hedge timing.
│
├── load_shed.rs       load_shed(should_shed, f) — free function predicate-based rejection.
│                      load_shed_with_policy_context() — cancellation/deadline-aware.
│
│  ── Infrastructure ─────────────────────────────────────────────────────────
│
├── pipeline.rs        PipelineBuilder<E> — collects steps, validates order on build().
│                      ResiliencePipeline<E> — executes steps recursively.
│                      Step<E> — LoadShed | RateLimiter | Timeout | Retry |
│                      CircuitBreaker | Bulkhead.
│
└── gate.rs            GateClosed — error when gate is already closing.
│                      GateCloseTimeout — bounded close diagnostic error.
│                      GateGuard — RAII exit token; returns permit on drop.
│                      Gate — cooperative shutdown barrier (Semaphore + AtomicBool).
```

---

## Data Flow

### Pipeline execution path

```text
Caller
  │  pipeline.call(|| async { ... })
  ▼
ResiliencePipeline::call
  │  run_steps(steps, idx=0, f)
  │
  ├── Step::LoadShed(predicate)
  │     predicate()? Err(LoadShed) : run_steps(idx+1)
  │
  ├── Step::RateLimiter(check)
  │     check()? run_steps(idx+1) : Err(RateLimited { retry_after })
  │
  ├── Step::Timeout(d)
  │     tokio::time::timeout(d, run_steps(steps, idx+1, f))
  │
  ├── Step::Retry(config)
  │     retry_with(inner_config, || classify_inner(run_steps(idx+1)))
  │     classify_inner: Ok→Ok, retryable error→Err(e), permanent error→return directly
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

```text
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
  │     └─ enough successes in HalfOpen → transition to Closed
  │     └─ sink.record(CircuitStateChanged { from, to })
  ▼
Result<T, CallError<E>>
```

### Retry execution path

```text
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
  │    4. Total budget exhausted before/during attempt or sleep → Err(CallError::Timeout(budget))
  ▼
Result<T, CallError<E>>
```
