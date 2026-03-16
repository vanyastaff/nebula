# nebula-resilience

Fault-tolerance patterns for Nebula services.

`nebula-resilience` gives workflow actions and service calls bounded failure handling —
circuit breaking, bounded retry, concurrency isolation, rate limiting, timeouts, graceful
degradation, and cooperative shutdown — with advanced Rust type-system guarantees:
const-generic configuration validation, phantom-type state safety, and zero-cost abstractions.

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
| **`CircuitBreaker`** | Tracks consecutive failures; fails-fast when threshold is crossed. Probes recovery via half-open state. Const-generic `FAILURE_THRESHOLD` and `RESET_TIMEOUT_MS`. |
| **`RetryStrategy`** | Bounded retry with configurable backoff (exponential, fixed, linear), jitter, and retryability condition. Returns `(result, RetryStats)` on success; `RetryFailure` on exhaustion. |
| **`Bulkhead`** | Semaphore-backed concurrency cap with optional bounded queue and acquire timeout. Emits `BulkheadFull` when at capacity. |
| **`RateLimiter`** | Token-bucket rate limiter. Controls maximum operations per time window across concurrent callers. |
| **`timeout`** | Hard async deadline. Returns `ResilienceError::Timeout` if the inner future exceeds the configured duration. |
| **`FallbackStrategy`** | Alternative result path when the primary fails. Built-in variants: `ValueFallback`, `FunctionFallback`, `CacheFallback`, `ChainedFallback`. |
| **`HedgeExecutor`** | Fires speculative parallel requests after a configurable delay. Returns the first success; reduces tail latency. |
| **`Gate` / `GateGuard`** | Cooperative shutdown barrier. `enter()` acquires an RAII guard; `close()` drains all in-flight guards before returning. |
| **`ResilienceManager`** | Central service registry. Registers typed services with named policies; dispatches typed and untyped executions. |
| **`LayerBuilder`** | Fluent composer. Stacks `timeout → bulkhead → circuit breaker → retry` into a `ResilienceChain` pipeline. |
| **`ResiliencePolicy`** | Named configuration set (retry + circuit breaker + bulkhead) attached to a service in `ResilienceManager`. |
| **`ResilienceError`** | Rich error enum. Every variant carries structured context, a `classify() → ErrorClass` method, and optional retry hints. |

---

## Quick Start

### Single pattern

```rust
use nebula_resilience::prelude::*;

// Compile-time validated configuration
let config = CircuitBreakerConfig::<5, 30_000>::new()
    .with_half_open_limit(3)
    .with_min_operations(10);

let breaker = CircuitBreaker::new(config)?;

let result = breaker.execute(|| async {
    // Your call here
    Ok::<_, ResilienceError>("response")
}).await;
```

### Composing patterns with `LayerBuilder`

```rust
use nebula_resilience::compose::LayerBuilder;
use std::time::Duration;

let chain = LayerBuilder::new()
    .with_timeout(Duration::from_secs(5))
    .with_retry_exponential(3, Duration::from_millis(100))
    .build();

let result = chain.execute(|| async {
    Ok::<_, ResilienceError>("ok")
}).await;
```

### Retry with stats

```rust
use nebula_resilience::{
    RetryConfig, ExponentialBackoff, ConservativeCondition, JitterPolicy, RetryStrategy,
};

let config = RetryConfig::new(
    ExponentialBackoff::<100, 20, 5000>::default(),
    ConservativeCondition::<3>::new(),
).with_jitter(JitterPolicy::Equal);

let strategy = RetryStrategy::new(config)?;

let (result, stats) = strategy
    .execute(|| async { Ok::<_, ResilienceError>("ok") })
    .await?;

println!("Succeeded after {} attempt(s)", stats.attempts);
```

### Cooperative shutdown with `Gate`

```rust
use nebula_resilience::gate::{Gate, GateClosed};

let gate = Gate::new();

// Each handler acquires a guard; work progresses while the guard is live.
let _guard = gate.enter().expect("gate is open");

// Shutdown: reject new entries, drain all outstanding guards.
// gate.close().await;
```

---

## Feature Matrix

| Feature | Type | Default |
|---------|------|---------|
| Circuit breaking with half-open probes | `CircuitBreaker<N, M>` | always on |
| Const-generic failure threshold | `CircuitBreakerConfig::<THRESHOLD, TIMEOUT_MS>` | always on |
| Exponential / fixed / linear backoff | `ExponentialBackoff`, `FixedDelay`, `LinearBackoff` | opt-in |
| Jitter policy (none / full / equal) | `JitterPolicy` | opt-in |
| Retry conditions (conservative / aggressive / time-based) | `ConservativeCondition`, `AggressiveCondition` | opt-in |
| Cancellation-aware retry | `execute_resilient_with_cancellation` | opt-in |
| Semaphore-bounded concurrency | `Bulkhead` | always on |
| Token-bucket rate limiting | `RateLimiter` | always on |
| Hard deadline timeout | `timeout()` | always on |
| Value / function / cache / chained fallback | `FallbackStrategy` impls | opt-in |
| Speculative parallel hedging | `HedgeExecutor`, `AdaptiveHedgeExecutor` | opt-in |
| Cooperative shutdown barrier | `Gate`, `GateGuard` | always on |
| Fluent layer composition | `LayerBuilder`, `ResilienceChain` | always on |
| Named service policies | `ResilienceManager`, `ResiliencePolicy` | always on |
| Typed event categories | `Event<C>`, `RetryEventCategory`, … | always on |
| `tracing` spans per execution | `SpanGuard`, `PatternSpanGuard` | always on |
| In-process metrics collection | `MetricsCollector`, `MetricSnapshot` | always on |
| Dynamic config reload | `DynamicConfig`, `DynamicConfigurable` | opt-in |

---

## Crate Layout

```
crates/resilience/
├── src/
│   ├── lib.rs                       public API, crate-level docs and examples
│   ├── core/
│   │   ├── error.rs                 ResilienceError, ErrorClass, ErrorContext
│   │   ├── result.rs                ResilienceResult, ResultExt
│   │   ├── traits.rs                Executable, HealthCheck, PatternMetrics, Retryable
│   │   ├── config.rs                ResilienceConfig, ConfigBuilder, nebula-config bridge
│   │   ├── types.rs                 newtypes: RetryCount, Timeout, MaxConcurrency, …
│   │   ├── cancellation.rs          CancellationContext, ShutdownCoordinator
│   │   ├── advanced.rs              typestate PolicyBuilder, ValidatedRetryConfig
│   │   ├── categories.rs            sealed Category, PatternCategory, ServiceCategory
│   │   ├── metrics.rs               MetricsCollector, MetricSnapshot, MetricKind
│   │   └── dynamic/                 runtime-mutable configuration
│   ├── patterns/
│   │   ├── circuit_breaker.rs       CircuitBreaker, CircuitBreakerConfig, CircuitState
│   │   ├── retry.rs                 RetryStrategy, RetryConfig, backoff policies, conditions
│   │   ├── bulkhead.rs              Bulkhead, BulkheadConfig, BulkheadStats
│   │   ├── rate_limiter/            RateLimiter, token-bucket implementation
│   │   ├── timeout.rs               timeout() async fn
│   │   ├── fallback.rs              FallbackStrategy, ValueFallback, FunctionFallback, …
│   │   └── hedge.rs                 HedgeExecutor, HedgeConfig, AdaptiveHedgeExecutor
│   ├── compose.rs                   LayerBuilder, ResilienceChain, ResilienceLayer, LayerStack
│   ├── gate.rs                      Gate, GateGuard, GateClosed
│   ├── manager.rs                   ResilienceManager, Service, Operation, typed execution
│   ├── policy.rs                    ResiliencePolicy, RetryPolicyConfig, PolicyMetadata
│   ├── retryable.rs                 Retryable trait blanket impls
│   ├── helpers.rs                   log_result! and print_result! macros
│   └── observability/
│       ├── mod.rs                   ObservabilityHook, PatternEvent re-exports
│       ├── hooks.rs                 ObservabilityHooks, LoggingHook, MetricsHook, typed events
│       └── spans.rs                 SpanGuard, PatternSpanGuard, create_span
├── benches/                         Criterion benchmark suites
├── tests/                           integration tests
└── examples/                        end-to-end usage examples
```

---

## Documentation

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | Design decisions, const-generic validation, type-state pattern, module map |
| [api-reference.md](api-reference.md) | Full public API reference with all types and signatures |
| [PATTERNS.md](PATTERNS.md) | Pattern guide: composition order, mermaid flows, tuning rules |
| [composition.md](composition.md) | `LayerBuilder` fluent API and `ResilienceChain` execution model |
| [gate.md](gate.md) | `Gate` / `GateGuard` cooperative shutdown barrier |
| [observability.md](observability.md) | Events, metrics, hooks, and tracing spans |
| [RELIABILITY.md](RELIABILITY.md) | Failure modes, incident triage flow, reliability control loop |
| [MIGRATION.md](MIGRATION.md) | Breaking changes and upgrade paths |
