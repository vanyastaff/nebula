# Changelog

All notable changes to `nebula-resilience` will be documented in this file.

`nebula-resilience` is an internal Nebula workspace crate (`publish = false`).
Its version follows the workspace version, and compatibility expectations are
managed inside the Nebula repository rather than through crates.io releases.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-05-05

Initial implementation of the internal Nebula resilience layer.

### Added

#### Pipeline API

- Added `ResiliencePipeline<E>` and `PipelineBuilder<E>` for composing outbound-call
  protection policies in a single typed execution path.
- Added strict and ergonomic build modes:
  - `build_checked()` rejects unsafe policy ordering;
  - `build()` preserves builder ergonomics while warning on suspicious order;
  - `build_recommended_order()` encodes the default Nebula order.
- Added context-aware pipeline execution with shared cancellation, deadline, and scope
  propagation through `PolicyContext`.
- Added fallback-aware pipeline calls that preserve primary and fallback failures where
  graceful degradation itself fails.

#### Error Model

- Added `CallError<E>` as the crate-wide result error, preserving the caller's original
  operation error type without forced conversion into a resilience-specific enum.
- Added typed variants for resilience failures: open circuit, full bulkhead, timeout,
  exhausted retries, cancellation, load shedding, rate limiting, and fallback failure.
- Added `CallErrorKind` for low-cardinality dispatch, telemetry, fallback routing, and
  event payloads.
- Integrated with `nebula-error::Classify` so retry and circuit breaker behavior can
  distinguish transient, permanent, timeout, cancellation, overload, unavailable, and
  unknown failures.

#### Retry

- Added bounded retry execution with `RetryConfig`, `retry_with`, and per-error
  classification.
- Added backoff strategies:
  - fixed delay;
  - linear delay;
  - exponential delay with capped growth;
  - Fibonacci delay;
  - custom inline delay sequences.
- Added jitter policy support, retry hooks, retry-attempt events, and total retry budgets
  that bound both operation attempts and sleeps.

#### Circuit Breaker

- Added closed/open/half-open circuit breaker state machine.
- Added configurable failure thresholds, reset timeout, half-open probe limits, and
  successful-probe recovery thresholds.
- Added optional slow-call tracking, failure-rate thresholds, count-based sliding windows,
  dynamic break-duration escalation, and timeout classification controls.
- Added cancellation-safe half-open probe accounting so dropped futures do not leak probe
  capacity.

#### Bulkhead

- Added semaphore-backed concurrency isolation with `Bulkhead` and `BulkheadConfig`.
- Added fail-fast and queued acquisition modes, queue timeout support, and explicit
  `BulkheadPermit` handling.
- Added cancellation-safe queue accounting so dropped waiting futures release their queue
  slot correctly.

#### Rate Limiting

- Added `RateLimiter` and object-safe `ErasedRateLimiter` surfaces.
- Added built-in limiters:
  - `TokenBucket`;
  - `LeakyBucket`;
  - `SlidingWindow`;
  - `AdaptiveRateLimiter`.
- Added retry-after hints for rate-limit failures and context-aware acquisition paths for
  cancellation/deadline composition.

#### Timeout, Load Shedding, and Deadlines

- Added standalone timeout helpers and `TimeoutExecutor`.
- Added context-aware timeout execution that composes local timeouts with shared
  `PolicyContext` deadlines and cancellation.
- Added predicate-based load shedding with sink-integrated and context-aware variants.
- Added `Deadline` as a monotonic budget helper for policies that need remaining-time
  semantics.

#### Fallback and Hedging

- Added `FallbackStrategy<T>` plus value, function, cache, chain, priority, and
  operation-level fallback implementations.
- Added fallback lifecycle events for attempted, succeeded, and failed recovery paths.
- Added hedged execution for duplicate-safe/idempotent operations with configurable hedge
  delay, maximum duplicate requests, and exponential hedge-delay growth.
- Added adaptive hedge execution and latency tracking for tail-latency-sensitive calls.

#### Policy Context and Load Signals

- Added `PolicyContext` for passing cancellation, deadline, and low-cardinality scope
  across standalone policy calls and composed pipelines.
- Added `PolicySource<C>` so static and adaptive policy configuration can share one
  retrieval interface.
- Added `LoadSignal`, `LoadSnapshot`, and `ConstantLoad` for adaptive load-shedding and
  rate-limiting decisions.
- Added validation for load factors and error rates to keep adaptive decisions within
  finite `0.0..=1.0` bounds.

#### Observability

- Added `MetricsSink` as the crate-local observability extension point.
- Added `NoopSink` for zero-cost default operation and `RecordingSink` for tests.
- Added typed resilience events for circuit transitions, retry attempts, bulkhead
  rejection, timeout, hedge firing, rate limiting, load shedding, fallback lifecycle, and
  pipeline completion.
- Added `PolicyScope`, `ScopeValue`, `PipelineOutcome`, `CircuitState`, and
  `ResilienceEventKind` for low-cardinality metrics and event filtering.

#### Features and Serialization

- Added default `serde` feature for stable config/value boundary types, including configs,
  error/event discriminants, policy scopes, pipeline outcomes, stats snapshots, and load
  snapshots.
- Added validated deserialization for `LoadSnapshot` and `ConstantLoad`, preserving finite
  `0.0..=1.0` invariants for external config/event inputs.
- Added `full` as the convenience feature set for normal optional crate features.
- Added `loom` feature for model-checking selected atomic invariants with
  `RUSTFLAGS="--cfg loom"`.

#### Documentation and Verification

- Added crate README and documentation index covering purpose, workspace role, feature
  flags, API entry points, examples, and verification commands.
- Added API reference documentation for the resilience surface.
- Added tests for retry behavior, circuit breaker lifecycle, cancellation safety,
  fallback behavior, policy context contracts, rate limiter expiry, stress scenarios,
  property-tested backoff behavior, and serde round trips for boundary types.
