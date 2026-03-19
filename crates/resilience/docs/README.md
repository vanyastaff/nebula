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
| **`CallError<E>`** | Unified error returned by every pattern. `E` is the caller's own error type. Pattern errors (`CircuitOpen`, `BulkheadFull`, `Timeout`, `RetriesExhausted`, `LoadShed`, `RateLimited`, `Cancelled`) are separate enum variants. |
| **`ResiliencePipeline<E>`** | Composed middleware chain built via `PipelineBuilder`. Applies steps in order: first added = outermost. Recommended: `timeout → retry → circuit_breaker → bulkhead`. |
| **`CircuitBreaker`** | Tracks consecutive failures; fails-fast when `failure_threshold` is crossed. Probes recovery via half-open state. Plain-struct config, injectable `Clock` and `MetricsSink`. |
| **`retry` / `retry_with`** | Bounded retry with `BackoffConfig` enum (`Fixed`, `Linear`, `Exponential`), optional `JitterConfig`, and a predicate `retry_if`. Returns `CallError::RetriesExhausted` on exhaustion. |
| **`Bulkhead`** | Semaphore-backed concurrency cap. Returns `CallError::BulkheadFull` when at capacity. |
| **`RateLimiter`** | Trait implemented by `TokenBucket`, `LeakyBucket`, `SlidingWindow`, `AdaptiveRateLimiter`. Returns `CallError::RateLimited`. Also available as `AnyRateLimiter` for object-safe boxing. |
| **`timeout_fn`** | Hard async deadline. Returns `CallError::Timeout` if the future exceeds the duration. |
| **`load_shed`** | Free function. Returns `CallError::LoadShed` immediately when a predicate fires. Integrates with `LoadSignal` for adaptive shedding. |
| **`FallbackStrategy<T>`** | Alternative result path on failure. Built-in: `ValueFallback`. Custom via trait impl. |
| **`HedgeExecutor`** | Fires speculative parallel requests after `hedge_delay`. Returns first success; reduces tail latency. |
| **`Gate` / `GateGuard`** | Cooperative shutdown barrier. `enter()` acquires an RAII guard; `close()` drains all in-flight guards before returning. |
| **`MetricsSink`** | Observability extension point — receives `ResilienceEvent` values. Default: `NoopSink`. Test: `RecordingSink`. |
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
use nebula_resilience::signals::{LoadSignal, ConstantLoad};

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
| Composed middleware pipeline | `ResiliencePipeline<E>`, `PipelineBuilder<E>` | Ordered steps, order-validation warning |
| Circuit breaking with half-open probes | `CircuitBreaker`, `CircuitBreakerConfig` | Plain struct config, injectable clock/sink |
| Token-bucket rate limiting | `TokenBucket` | Capacity + refill rate |
| Leaky-bucket rate limiting | `LeakyBucket` | Constant leak rate |
| Sliding-window rate limiting | `SlidingWindow` | Time-window counter |
| Adaptive rate limiting | `AdaptiveRateLimiter` | Adjusts based on error rates |
| Object-safe rate limiter | `AnyRateLimiter` | `dyn RateLimiter` wrapper |
| Exponential / fixed / linear backoff | `BackoffConfig` enum | No const generics |
| Jitter policy (none / full) | `JitterConfig` | Optional fraction |
| Predicate-driven retry | `RetryConfig::retry_if` | Per-error-type classification |
| Cancellation-aware retry | `CancellationContext` | `CancellableFuture` combinator |
| Semaphore-bounded concurrency | `Bulkhead`, `BulkheadConfig` | |
| Hard deadline timeout | `timeout_fn`, `TimeoutExecutor` | |
| Value fallback | `ValueFallback<T>` | Returns cloned constant |
| Custom fallback | `FallbackStrategy<T>` trait | Implement for custom logic |
| Speculative parallel hedging | `HedgeExecutor`, `HedgeConfig` | Reduces tail latency |
| Load shedding | `load_shed` free function | Predicate-based |
| Cooperative shutdown barrier | `Gate`, `GateGuard` | |
| Metrics sink | `MetricsSink` trait, `NoopSink`, `RecordingSink` | Receives `ResilienceEvent` |
| Adaptive config source | `PolicySource<C>` trait | Blanket impl for static configs |
| Runtime load signals | `LoadSignal` trait, `ConstantLoad` | For adaptive policies |
| Injectable clock | `Clock` trait, `SystemClock` | Enables deterministic tests |
| `tracing` spans | `SpanGuard`, `PatternSpanGuard<C>` | RAII span with success/error recording |

---

## Crate Layout

```
crates/resilience/
├── src/
│   ├── lib.rs                   public API, re-exports, crate-level docs
│   │
│   │   ── Core types ───────────────────────────────────────────────────
│   ├── types.rs                 CallError<E>, CallResult<T,E>, ConfigError
│   ├── error.rs                 ResilienceError (internal), ErrorClass, ErrorContext
│   ├── result.rs                ResilienceResult<T>, ResultExt
│   ├── cancellation.rs          CancellationContext, CancellableFuture, ShutdownCoordinator
│   ├── policy_source.rs         PolicySource<C> trait + blanket impl
│   ├── signals.rs               LoadSignal trait, ConstantLoad
│   ├── clock.rs                 Clock trait, SystemClock
│   ├── metrics.rs               MetricsCollector, MetricSnapshot, MetricKind, Metrics
│   │
│   │   ── Observability ─────────────────────────────────────────────────
│   ├── sink.rs                  MetricsSink, NoopSink, RecordingSink, ResilienceEvent,
│   │                            CircuitState
│   ├── hooks.rs                 ObservabilityHooks, ObservabilityHook, PatternEvent,
│   │                            Event<C>, EventCategory, LoggingHook, MetricsHook,
│   │                            LogLevel, typed event category markers
│   ├── spans.rs                 SpanGuard, PatternSpanGuard<C>, create_span,
│   │                            record_success, record_error
│   │
│   │   ── Patterns ─────────────────────────────────────────────────────
│   ├── circuit_breaker.rs       CircuitBreaker, CircuitBreakerConfig, Outcome
│   ├── retry.rs                 RetryConfig<E>, BackoffConfig, JitterConfig,
│   │                            retry(), retry_with()
│   ├── bulkhead.rs              Bulkhead, BulkheadConfig
│   ├── rate_limiter.rs          RateLimiter trait, TokenBucket, LeakyBucket,
│   │                            SlidingWindow, AdaptiveRateLimiter, AnyRateLimiter
│   ├── timeout.rs               timeout_fn(), timeout_with_original_error(),
│   │                            TimeoutExecutor
│   ├── fallback.rs              FallbackStrategy<T>, ValueFallback,
│   │                            AnyStringFallbackStrategy
│   ├── hedge.rs                 HedgeExecutor, HedgeConfig
│   ├── load_shed.rs             load_shed() free function
│   ├── retryable.rs             Retryable trait + blanket impls
│   │
│   │   ── Infrastructure ───────────────────────────────────────────────
│   ├── pipeline.rs              ResiliencePipeline<E>, PipelineBuilder<E>
│   ├── gate.rs                  Gate, GateGuard, GateClosed
│   └── helpers.rs               log_result! and print_result! macros
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
