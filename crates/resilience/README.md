---
name: nebula-resilience
role: Stability Patterns Pipeline (Circuit Breaker + Timeout + Retry-with-Backoff composition)
status: stable
last-reviewed: 2026-04-17
canon-invariants: [L2-11.2]
related: [nebula-error, nebula-action]
---

# nebula-resilience

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
inside actions. Pattern: *Circuit Breaker + Timeout + Retry-with-Backoff* composition (Release It!;
`docs/GLOSSARY.md` Architectural Patterns). Per canon §11.2, this is the **canonical retry surface
today** — engine-level node re-execution from an `ActionResult::Retry` variant is `planned`, not
yet implemented.

## Public API

- `ResiliencePipeline<E>` — composable pipeline: `.classifier()`, `.classify_errors()`, `.with_sink()`, `.timeout()`, `.retry()`, `.circuit_breaker()`, `.bulkhead()`, `.rate_limiter()` / `.rate_limiter_from()`, `.load_shed()`, then `.build()`. Hedging stays in the `hedge` module (no `.hedge()` builder step on the pipeline). For graceful degradation after the pipeline returns, use `ResiliencePipeline::call_with_fallback` (separate from the builder).
- `CallError<E>` — wrapper error returned by all pipeline calls; no type erasure, no forced mapping.
- `retry::RetryConfig`, `retry::BackoffConfig`, `retry::retry_with` — standalone retry with `Classify`-aware error filtering.
- `circuit_breaker::CircuitBreaker`, `circuit_breaker::CircuitBreakerConfig` — half-open/open/closed state machine.
- `bulkhead::Bulkhead`, `bulkhead::BulkheadConfig` — concurrency-limiting bulkhead.
- `rate_limiter::RateLimiter` (+ optional `governor` feature for GCRA algorithm).
- `timeout::{timeout, TimeoutExecutor}` and `load_shed::{load_shed, load_shed_with_sink}` — standalone timeout / load-shed combinators.
- `fallback::{FallbackStrategy, ValueFallback, FunctionFallback, CacheFallback, ChainFallback, PriorityFallback, FallbackOperation}` — graceful degradation strategies.
- `hedge::{HedgeConfig, HedgeExecutor, AdaptiveHedgeExecutor}` — speculative execution (hedged requests).
- `sink::{MetricsSink, ResilienceEvent, ResilienceEventKind, RecordingSink}` — observability hooks for pipeline and pattern events.

## Contract

- **[L2-§11.2]** This crate is the **canonical retry surface for outbound calls inside an action**. Engine-level node re-execution with persisted attempt accounting is `planned`; until that row moves to `implemented`, no public API may describe engine-level retry as a current capability. Seam: action call sites that compose `ResiliencePipeline`. Test coverage: see `docs/MATURITY.md`.
- **[L1-§4.2]** Retry filtering is driven by `nebula-error::Classify::retry_hint()` — transient vs permanent is an explicit classification, not folklore in individual action bodies.
- **[L1-§4.3]** This crate is listed in the canon architecture table as the *Keep-alive + Safety* pillar implementation.

## Non-goals

- Not an engine-level retry scheduler — the engine orchestrating node re-execution with persisted attempt accounting is a separate `planned` capability (see canon §11.2).
- Not a durable control plane — in-process patterns only; durable cancel/dispatch lives in `execution_control_queue` (canon §12.2, §4.5).
- Not a metrics export layer — resilience events feed `nebula-metrics` via observability hooks, not the reverse.

## Maturity

See `docs/MATURITY.md` row for `nebula-resilience`.

- API stability: `stable` — `ResiliencePipeline`, `RetryConfig`, `CircuitBreaker`, and `CallError` are in active use; benchmarks cover all seven patterns.
- `MetricsSink`-based observability hooks and hedge-related APIs are newer and may still get minor refinements.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.2 (Safety pillar / ErrorClassifier), §4.3 (Keep-alive), §6 (architecture ↔ pillars table), §11.2 (retry contract table).
- Glossary: `docs/GLOSSARY.md` Architectural Patterns (*Circuit Breaker + Timeout + Retry-with-Backoff*, Release It!).
- Siblings: `nebula-error` (provides `Classify` / `RetryHint`), `nebula-action` (primary consumer).

## Appendix: Crate-local guides

Extended documentation lives in `crates/resilience/docs/`:

- `README.md` — overview and pattern guide
- `docs/README.md` — overview and feature matrix
- `api-reference.md` — full API surface reference
- `composition.md` — pipeline composition guide
- `observability.md` — observability hooks
- `gate.md` — cooperative shutdown barrier
- `architecture.md` — internal architecture notes

```bash
# Verify locally
cargo check -p nebula-resilience --all-features
cargo test -p nebula-resilience
cargo bench -p nebula-resilience
```
