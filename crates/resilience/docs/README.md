# nebula-resilience

Fault-tolerance patterns for Nebula services.

`nebula-resilience` gives workflow actions and service calls bounded failure handling —
circuit breaking, bounded retry, concurrency isolation, rate limiting, timeouts, load
shedding, graceful degradation, and cooperative shutdown.

All patterns return `CallError<E>`, preserving the caller's own error type without
forcing conversions into a resilience-specific error enum.

---

## Table of Contents

- [Core Concepts](#core-concepts)
- [Quick Start](#quick-start)
- [Feature Matrix](#feature-matrix)
- [Crate Layout](#crate-layout)
- [Documentation](#documentation)

---

## Core Concepts

| Concept | Description |
|---------|-------------|
| **`CallError<E>`** | Unified error returned by every pattern. `E` is the caller's own error type. Pattern errors (`CircuitOpen`, `BulkheadFull`, `Timeout`, `RetriesExhausted`, `LoadShed`, `RateLimited`, `Cancelled`, `FallbackFailed`) are separate enum variants. `FallbackFailedWithContext` preserves both primary and fallback failures where available. Includes `flat_map_inner()` helper. |
| **`PolicyContext`** | Shared execution context carrying cancellation, deadline, and low-cardinality scope for a protected call. Pipeline, bulkhead, rate limiter, timeout, load-shed, circuit breaker, and fallback-operation entry points can consume it. |
| **`ResiliencePipeline<E>`** | Composed middleware chain built via `PipelineBuilder`. Applies steps in order: first added = outermost. Recommended: `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`. `build_checked()` rejects unsafe order. `call_with_policy_context()` and `call_with_policy_context_and_fallback()` propagate cancellation/deadline/scope through the call; cancellation-only helpers remain available. |
| **`CircuitBreaker`** | Tracks consecutive failures; fails-fast when `failure_threshold` is crossed. Probes recovery via half-open state. Plain-struct config, injectable `Clock` and `MetricsSink`. |
| **`retry` / `retry_with`** | Bounded retry with `BackoffConfig` enum (`Fixed`, `Linear`, `Exponential`), optional `JitterConfig`, and a predicate `retry_if`. Returns `CallError::RetriesExhausted` on exhaustion. |
| **`Bulkhead`** | Semaphore-backed concurrency cap. Returns `CallError::BulkheadFull` when at capacity. |
| **`RateLimiter` / `ErasedRateLimiter`** | Static-dispatch trait implemented by `TokenBucket`, `LeakyBucket`, `SlidingWindow`, `AdaptiveRateLimiter`, plus an object-safe facade for heterogeneous registries. Returns `CallError::RateLimited`. |
| **`timeout`** | Hard async deadline. Returns `CallError::Timeout` if the future exceeds the duration. Context-aware helpers compose with workflow cancellation/deadline. |
| **`load_shed`** | Free function. Returns `CallError::LoadShed` immediately when a predicate fires. Context-aware helpers avoid evaluating predicates after cancellation/deadline expiry. Integrates with `LoadSignal` for adaptive shedding. |
| **`FallbackStrategy<T>`** | Alternative result path on failure. Built-ins include value, function, cache, chain, and priority strategies. Custom strategies implement recovery while the safe `fallback()` wrapper checks whether the error class is recoverable. By default recovers operation failures, retry exhaustion, timeout, and open circuit, but not cancellation or overload rejections. |
| **`HedgeExecutor`** | Fires speculative parallel requests after `hedge_delay`. Duplicate requests are disabled by default and require `HedgeSafety::Idempotent`; losing or cancelled call-owned tasks are aborted. |
| **`Gate` / `GateGuard`** | Cooperative shutdown barrier. `enter()` acquires an RAII guard; `close()` drains all in-flight guards before returning. `close_with_timeout()` returns a typed timeout with active guard count. |
| **`Deadline`** | Shared monotonic time-budget helper used by policies that need remaining-budget semantics. |
| **`MetricsSink`** | Observability extension point — receives `ResilienceEvent` values, including scoped `PipelineCompleted` outcomes. Default: `NoopSink`. Test: `RecordingSink`. |
| **`PolicySource<C>`** | Trait for adaptive config. Blanket impl makes any `Clone + Send + Sync` value a static source. |
| **`LoadSignal`** | Runtime signal providing `load_factor`, `error_rate`, `p99_latency` for adaptive policies. `ConstantLoad` for testing. |

---

## Quick Start

### Single pipeline

```rust
use nebula_resilience::{ResiliencePipeline, CallError};
use nebula_resilience::retry::{RetryConfig, BackoffConfig};
use std::time::Duration;

let pipeline = ResiliencePipeline::<MyError>::builder()
    .timeout(Duration::from_secs(5))
    .retry(RetryConfig::new(3)?.backoff(BackoffConfig::exponential_default()))
    .build();

let result = pipeline.call(|| Box::pin(async {
    Ok::<_, MyError>("success")
})).await;
```

### Circuit breaker standalone

```rust
use nebula_resilience::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use std::time::Duration;

let cb = CircuitBreaker::new(CircuitBreakerConfig {
    failure_threshold: 5,
    reset_timeout: Duration::from_secs(30),
    ..Default::default()
})?;

let result = cb.call(|| Box::pin(async {
    Ok::<_, MyError>("response")
})).await;
```

### Retry with predicate

```rust
use nebula_resilience::retry::{RetryConfig, BackoffConfig, retry_with};

let config = RetryConfig::<MyError>::new(3)?
    .backoff(BackoffConfig::Fixed(Duration::from_millis(50)))
    .retry_if(|e| e.is_transient());

let result = retry_with(config, || Box::pin(async {
    Ok::<_, MyError>(42)
})).await;
```

### Load shedding

```rust
use nebula_resilience::load_shed;
use nebula_resilience::policy::{LoadSignal, ConstantLoad};

let signal = ConstantLoad::idle();

let result = load_shed(
    || signal.load_factor() > 0.9,
    || Box::pin(async { Ok::<_, MyError>(42) }),
).await;
```

### Cooperative shutdown with `Gate`

```rust
use nebula_resilience::gate::{Gate, GateClosed};

let gate = Gate::new();

let _guard = gate.enter().expect("gate is open");

// On shutdown:
// gate.close().await;
```

---

## Feature Matrix

| Feature | Type | Notes |
|---------|------|-------|
| Generic error type preservation | `CallError<E>` | No forced mapping to resilience error |
| Composed middleware pipeline | `ResiliencePipeline<E>`, `PipelineBuilder<E>` | Ordered steps, `build_checked()` strict validation, order-warning `build()` |
| Circuit breaking with half-open probes | `CircuitBreaker`, `CircuitBreakerConfig` | Plain struct config, injectable clock/sink |
| Token-bucket rate limiting | `TokenBucket` | Capacity + refill rate |
| Leaky-bucket rate limiting | `LeakyBucket` | Constant leak rate |
| Sliding-window rate limiting | `SlidingWindow` | Time-window counter |
| Adaptive rate limiting | `AdaptiveRateLimiter` | Adjusts based on error rates |
| Exponential / fixed / linear backoff | `BackoffConfig` enum | Serde support |
| Jitter policy (none / full) | `JitterConfig` | Optional fraction |
| Predicate-driven retry | `RetryConfig::retry_if` | Per-error-type classification |
| Cancellation-aware retry | `CancellationContext` | `CancellableFuture` combinator |
| Shared policy context | `PolicyContext` | Carries cancellation, deadline, and scope across pipeline and standalone policy calls |
| Semaphore-bounded concurrency | `Bulkhead`, `BulkheadConfig` | Serde support |
| Hard deadline timeout | `timeout`, `TimeoutExecutor` | `try_new()` rejects zero config; context-aware calls compose with `PolicyContext` |
| Value fallback | `ValueFallback<T>` | Returns cloned constant |
| Custom fallback | `FallbackStrategy<T>` trait | Implement recovery for custom logic; keep `fallback()` as the safe entry point |
| Speculative parallel hedging | `HedgeExecutor`, `AdaptiveHedgeExecutor`, `HedgeConfig`, `HedgeSafety` | Reduces tail latency for idempotent operations. Constructor returns `Result`. Serde on `HedgeConfig`. |
| Load shedding | `load_shed` free function | Predicate-based, with context-aware variants |
| Cooperative shutdown barrier | `Gate`, `GateGuard` | Bounded close available via `close_with_timeout()` |
| Metrics sink | `MetricsSink` trait, `NoopSink`, `RecordingSink` | Receives `ResilienceEvent`. `ResilienceEventKind` enum for counting. |
| Scoped pipeline outcomes | `PolicyScope`, `PipelineOutcome` | Distinguishes primary success/failure and fallback recovery. |
| Shared deadline helper | `Deadline` | Bounds attempts and sleeps by remaining budget. |
| Adaptive config source | `PolicySource<C>` trait | Blanket impl for static configs (in `policy` module) |
| Runtime load signals | `LoadSignal` trait, `ConstantLoad` | For adaptive policies (in `policy` module) |
| Injectable clock | `Clock` trait, `SystemClock` | Enables deterministic tests |

---

## Crate Layout

```
crates/resilience/
├── src/
│   ├── lib.rs                   public API, re-exports, crate-level docs
│   │
│   │   ── Core types ───────────────────────────────────────────────────
│   ├── error.rs                 CallError<E>, CallErrorKind, CallResult<T,E>, ConfigError
│   ├── cancellation.rs          CancellationContext, CancellableFuture
│   ├── context.rs               PolicyContext
│   ├── deadline.rs              Deadline
│   ├── policy.rs                PolicySource<C> trait + blanket impl,
│   │                            LoadSignal trait, ConstantLoad
│   ├── clock.rs                 Clock trait, SystemClock
│   │
│   │   ── Observability ─────────────────────────────────────────────────
│   ├── sink.rs                  MetricsSink, NoopSink, RecordingSink, ResilienceEvent,
│   │                            ResilienceEventKind, CircuitState, PolicyScope,
│   │                            PipelineOutcome
│   │
│   │   ── Patterns ─────────────────────────────────────────────────────
│   ├── circuit_breaker.rs       CircuitBreaker, CircuitBreakerConfig, Outcome
│   ├── retry.rs                 RetryConfig<E>, BackoffConfig, JitterConfig,
│   │                            retry(), retry_with()
│   ├── bulkhead.rs              Bulkhead, BulkheadConfig
│   ├── rate_limiter.rs          RateLimiter trait, TokenBucket, LeakyBucket,
│   │                            SlidingWindow, AdaptiveRateLimiter
│   ├── timeout.rs               timeout(), TimeoutExecutor,
│   │                            context-aware timeout helpers
│   ├── fallback.rs              FallbackStrategy<T>, ValueFallback
│   ├── hedge.rs                 HedgeExecutor, AdaptiveHedgeExecutor, HedgeConfig,
│   │                            HedgeSafety
│   ├── load_shed.rs             load_shed() free function,
│   │                            context-aware load shedding
│   │
│   │   ── Infrastructure ───────────────────────────────────────────────
│   ├── pipeline.rs              ResiliencePipeline<E>, PipelineBuilder<E>
│   └── gate.rs                  Gate, GateGuard, GateClosed,
│                                GateCloseTimeout
├── benches/                     Criterion benchmark suites
├── tests/                       integration tests
└── examples/                    end-to-end usage examples
```

---

## Documentation

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | Design decisions, `CallError<E>` model, pipeline internals, module map |
| [api-reference.md](api-reference.md) | Full public API reference with all types and signatures |
| [composition.md](composition.md) | `PipelineBuilder` / `ResiliencePipeline` composition model |
| [gate.md](gate.md) | `Gate` / `GateGuard` cooperative shutdown barrier |
| [observability.md](observability.md) | `MetricsSink`, `ResilienceEvent`, hooks, and tracing spans |
