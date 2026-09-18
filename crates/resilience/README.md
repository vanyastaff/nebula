---
name: nebula-resilience
role: Stability Patterns Pipeline (Circuit Breaker + Timeout + Retry-with-Backoff composition)
status: stable
last-reviewed: 2026-09-17
canon-invariants: [L2-11.2]
related: [nebula-error, nebula-action]
---

# nebula-resilience

Internal Nebula workspace crate. It is not published as a standalone public crate; its
versioning, documentation, and compatibility expectations follow the Nebula repository.

The crate's reference documentation is its rustdoc: every public item carries its contract,
and the doc examples compile as doctests. This file is the map, not a second copy.

## Purpose

Actions that call external APIs face flaky networks, rate limits, and transient failures. Without
a shared resilience layer, each action author re-implements retry loops, circuit breakers, and
timeout logic inconsistently — some retry permanent errors, others do not retry transient ones.
`nebula-resilience` provides a composable pipeline of seven patterns (retry, circuit breaker,
timeout, bulkhead, rate limiter, fallback, hedge) that action authors wire at outbound call sites.
The patterns share `nebula-error`'s `Classify` trait to distinguish transient from permanent errors
automatically.

## Role

**Stability Patterns Pipeline** — the canonical in-process fault-tolerance layer for outbound calls
inside actions. Pattern: *Circuit Breaker + Timeout + Retry-with-Backoff* composition (Release It!).
Per canon §11.2 this crate is the **only retry surface**
in the workflow stack: engine node re-execution is operator-declared policy, and retry semantics
for transient in-action failures compose inside the action.

## Cargo Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `serde` | yes | Serde support for config/value boundary types: configs, error/event discriminants, policy scopes, pipeline outcomes, stats, and load snapshots. |
| `bench-internals` | no | Exposes internal helpers (`retry_with_inner`, `LatencyTracker`) that the criterion benches measure directly. Adds visibility only, never behavior; not part of the documented surface. |

The crate intentionally does not expose optional third-party limiter wrappers. Built-in rate
limiters live in `rate_limiter.rs`; specialized external adapters should stay at integration
boundaries.

Runtime executors, guards, sinks, callbacks, and generic caller errors intentionally stay outside
serde because they carry live process state or user-owned types, not stable Nebula config/event
data.

## Terminology

Our names follow the source each pattern already cites; the column on the right
lists the synonyms `doc(alias)` makes searchable in rustdoc, so a reader
arriving with another library's vocabulary lands on the right item.

| Our name | Industry synonyms (aliases) | Standard |
|---|---|---|
| `failure_threshold` | `failureRateThreshold` | Resilience4j |
| `reset_timeout` | `waitDurationInOpenState`, `breakDuration`, `sleepWindow` | Resilience4j / Polly / Hystrix |
| `max_half_open_operations` | `permittedNumberOfCallsInHalfOpenState` | Resilience4j |
| `min_operations` | `minimumNumberOfCalls`, `minimumThroughput` | Resilience4j / Polly |
| `queue_wait_timeout` | `maxWaitDuration` | Resilience4j |
| `max_concurrency` | `maxConcurrentCalls` | Polly |
| `max_attempts` | `maxAttempts` | Resilience4j |
| `total_budget` | `totalTimeout`, `apiCallTimeout` | AWS SDK retry |
| `JitterConfig::Additive` | additive jitter | AWS Builders' Library |
| `RateLimiterStatus::remaining` / `limit_per_second` | `RateLimit-Remaining`, quota | RFC 9110 (`RateLimit` fields) |
| `InstantSource` | `Clock`, `TimeSource` | Java `java.time.InstantSource` |
| `EventSink` | `MetricsSink` (former name), `EventExporter` | OpenTelemetry |
| `CallContext` | `PolicyContext` (former name) | — |
| `Idempotent` (hedge) | idempotent method | RFC 9110 |

The unaliased decisions are deliberate: `CircuitBreaker`, `Bulkhead`,
`RetryConfig`, `RateLimited`, and `Deadline` are already the standard terms
(Release It!, gRPC), and `CallError::Operation` keeps the caller's payload
rather than naming a foreign error taxonomy.

## Workspace API

Read the rustdoc of `src/lib.rs` for the exhaustive re-export surface. The entry points:

- `ResiliencePipeline<E>` / `PipelineBuilder<E>` — compose `.classify_errors()`, `.with_sink()`,
  `.scope()`, `.timeout()`, `.retry()`, `.circuit_breaker()`, `.bulkhead()`,
  `.rate_limiter_from()` / `.rate_limiter_erased()`, `.load_shed()`, then `build()` (warns on
  suboptimal order), `try_build()` (rejects it), or `build_sorted()` (sorts it).
  Call through `call()`, `call_with_context()`, or `call_with_context_and_fallback()`.
  Hedging is deliberately not a builder step (see `hedge` docs for why).
- `CallError<E>` — error of every pattern; carries the caller's `E` and a variant per rejection
  kind. `TaskPanicked` exists so a panicked attempt is never reported as cancellation.
- `retry::{RetryConfig, BackoffConfig, JitterConfig, retry, retry_with}` — `Classify`-aware retry.
- `circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState}` — one accounting model:
  consecutive-failure counters with leaky forgiveness, slow-call rate, configurable half-open
  recovery. `circuit_state()` is the read-only state accessor; `try_acquire()` mutates.
- `bulkhead::{Bulkhead, BulkheadConfig}` — semaphore + bounded queue.
- `rate_limiter::{RateLimiter, ErasedRateLimiter, TokenBucket, LeakyBucket, SlidingWindow,
  AdaptiveRateLimiter}` — `ErasedRateLimiter` is the object-safe facade for heterogeneous
  registries.
- `timeout::{timeout, timeout_with_context, TimeoutExecutor}`,
  `load_shed::{load_shed, load_shed_with_context[_and_sink]}`, `bulkhead.acquire_with_context` — standalone combinators.
- `fallback::{FallbackStrategy, ValueFallback, FunctionFallback, CacheFallback, ChainFallback,
  PriorityFallback, FallbackExecutor}` — one shared orchestration (`orchestrate_fallback`)
  serves both `FallbackExecutor` and the pipeline's `call_with_fallback*`, so the event contract
  cannot drift between them.
- `CallContext`, `Deadline` — the cancellation/deadline/scope contract of one call and its budget helper.
- `gate::{Gate, GateGuard, GateCloseTimeout}` — cooperative shutdown drain with a caller-chosen
  budget. In-process only; see `gate` docs for the drain contract.
- `hedge::{HedgeConfig, HedgeSafety, HedgeExecutor, AdaptiveHedgeExecutor}` — speculative
  duplication, restricted to duplicate-safe non-effecting calls.
- `events::{EventSink, EventScope, ScopeValue, ResilienceEvent, RecordingSink}` — observability
  hooks; the default is the zero-cost `NoopSink`. (`EventSink` was `MetricsSink`; it
  receives events, not metrics.)
- `policy::{PolicySource, LoadSignal, LoadSnapshot, ConstantLoad}` — adaptive-config seams
  with no in-repo consumer yet. They are seams an embedding host may wire; the pipeline and the
  engine do not read a `LoadSignal` today, so do not assume adaptive behavior from their presence.
- `clock::{InstantSource, SystemInstant, MockInstant}` — injectable monotonic time for the circuit breaker (`clock::Clock` was renamed to avoid colliding with `nebula_core::accessor::Clock` and `nebula_action::webhook::Clock`).

## Contract

- **[L2-§11.2]** This crate is the **only retry surface in the workflow stack**. Retry, circuit
  breaking, and timeout for in-action outbound calls live in `ResiliencePipeline`. The engine's
  operator-declared node retry is a separate, persisted layer; the two do not share authority.
- **[L1-§4.2]** Retry filtering is driven by `nebula-error::Classify::retry_hint()` — transient vs
  permanent is an explicit classification, not folklore in individual action bodies.
- Ambiguous remote effects are never retried through this crate's pipeline without the
  stable-key contract (canon §11.2–§11.3). The hedge pattern must not be applied to effecting
  calls at all: speculative duplication bypasses the effect driver's invocation accounting.

## Non-goals

- Not an engine-level retry scheduler — engine node re-execution is operator policy (canon §11.2);
  this crate retries around outbound calls inside one action attempt.
- Not a durable control plane — in-process patterns only; durable cancel/dispatch lives in
  `execution_control_queue` (canon §12.2).
- Not a metrics export layer — resilience events feed `nebula-metrics` via sinks, not the reverse.
- Not runtime-agnostic — the async surface is built on tokio (`tokio::time`, `select!`,
  `tokio-util` cancellation).

## Maturity

See the `nebula-resilience` row in `docs/MATURITY.md`.

- API stability: `stable` — `ResiliencePipeline`, `RetryConfig`, `CircuitBreaker`, and `CallError`
  are in active use by `nebula-engine`, `nebula-credential`, and `nebula-api`.
- Test coverage: nextest suites plus benchmarks for the seven patterns, stress tests at 5K–10K
  concurrent tasks, and property-based tests over backoff arithmetic.
- The hedge pattern and the adaptive rate limiter have no in-repo consumer yet; their contract is
  pinned by their own tests and their scope is documented on the types.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.2 (Safety pillar), §4.3 (Keep-alive), §11.2–§11.3.
- Siblings: `nebula-error` (`Classify` / `RetryHint`), `nebula-action` (primary consumer).

```bash
# Verify locally
cargo check -p nebula-resilience --all-features
cargo check -p nebula-resilience --all-targets --no-default-features
cargo nextest run -p nebula-resilience
cargo test -p nebula-resilience --doc
cargo bench -p nebula-resilience --features bench-internals
```
