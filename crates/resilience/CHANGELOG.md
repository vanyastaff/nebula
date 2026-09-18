# Changelog

All notable changes to `nebula-resilience` will be documented in this file.

`nebula-resilience` is an internal Nebula workspace crate (`publish = false`).
Its version follows the workspace version, and compatibility expectations are
managed inside the Nebula repository rather than through crates.io releases.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Renamed — terminology alignment

Names were aligned with the industry vocabulary each pattern already cites
(Release It!, AWS Builders' Library, RFC 9110, Resilience4j/Polly) and with
RFC 1574 summary conventions. The crate is unpublished, so no compatibility
shims are kept; serialized config field aliases preserve old JSON keys where a
field was renamed.

| Old | New | Reason |
|---|---|---|
| `JitterConfig::Full { factor }` | `JitterConfig::Additive { max_fraction }` | `base + rand(0, f*base)` is additive jitter; full jitter is `rand(0, base)` |
| `RateLimiter::current_rate() -> f64` | `RateLimiter::status() -> RateLimiterStatus` | the old method had four meanings across implementations; `remaining`/`limit_per_second` follow the `RateLimit` header vocabulary |
| `clock::{Clock, SystemClock, MockClock}` | `clock::{InstantSource, SystemInstant, MockInstant}` | avoids collision with `nebula_core::accessor::Clock` and `nebula_action::webhook::Clock`; matches `java.time.InstantSource` |
| `CircuitBreaker::with_clock` / `clock_now` | `with_instant_source` / `monotonic_now` | names the injected contract, not a second "clock" |
| `MetricsSink`, `src/sink.rs` | `EventSink`, `src/events.rs` | the sink receives events, not metrics |
| `PolicyScope` | `EventScope` | groups event attributes; "policy" already means adaptive config in this crate |
| `FallbackOperation` | `FallbackExecutor` | matches `HedgeExecutor`/`TimeoutExecutor`; it executes calls, it is not an operation |
| `CircuitBreakerConfig::{break_duration_multiplier, max_break_duration}` | `{reset_timeout_multiplier, max_reset_timeout}` | one "reset timeout" vocabulary (Release It!) |
| `BulkheadConfig::timeout` | `queue_wait_timeout` | it bounds queue wait, not the call |
| `PolicyContext` | `CallContext` | the context of one protected call |
| `flat_map_inner` | `flat_map_operation` | one error-mapping vocabulary in the crate |
| `timeout_with_policy_context[_and_sink]` · `load_shed_with_policy_context[_and_sink]` · `acquire_with_policy_context` · `call_with_classifier_and_policy_context` | same names with `_with_context` | one context axis across the pipeline, combinator, and trait methods |
| `build_checked` / `build_recommended_order` | `try_build` / `build_sorted` | Rust `try_` convention for fallible construction; the other name says what it does |

### Added

- `RateLimiter::status() -> RateLimiterStatus` replacing the four-meanings
  `current_rate()`; `ErasedRateLimiter::status_boxed` is the object-safe twin.
- `CallError::TaskPanicked`, so a panicked hedge attempt is representable
  without being reported as cancellation.
- `bench-internals` feature: exposes `retry_with_inner` and `LatencyTracker`
  for the criterion benches without putting them on the documented surface.
  The crate's own integration effects (server/worker drains, engine limiter
  sharing) are recorded in the repository CHANGELOG.

### Changed

- Rustdoc is now the reference documentation: all summary lines are
  third person, every fallible item carries `# Errors`, every `pub async fn`
  carries a `# Cancel safety` section, and every public type links to an
  example (RFC 1574 / Rust API Guidelines).
- `doc(alias)` on the config and seam types maps the crate's names to the
  industry synonyms (Resilience4j, Polly, AWS SDK, RFC 9110); `README.md`
  carries the full table.
- Module layout: `rate_limiter/` is now one module per algorithm plus the
  trait; `pipeline/` is split into `builder.rs` and `executor.rs`.

### Removed

- The `docs/` prose folder. It documented APIs that no longer existed
  (`close_with_timeout()`, old `Outcome`/`BackoffConfig` shapes) and a `Gate`
  consumer that did not exist; the rustdoc and doctests replace it.
- The cancellation-only `call_with_context` / `call_with_context_and_fallback`
  pipeline methods. The context-taking methods (`call_with_policy_context*`)
  were renamed to those names, so `ResiliencePipeline` now has one context
  axis and four call methods instead of six with two meanings of "context".
- `CancellationContext::call` / `call_with_timeout` — replaced by the composable
  `timeout_with_context` / `bulkhead.acquire_with_context` paths.
- The unconsumed loom harness and its `loom` feature; the dead `full` feature
  alias (it only re-enabled the default `serde`).
- The `sliding_window_size` / `failure_rate_threshold` circuit-breaker config
  and its `OutcomeWindow` (one accounting model remains).

### Fixed

- A panicking hedge attempt is reported as `CallError::TaskPanicked` instead of
  being masked as cancellation.
- Server and worker shutdown drains are bounded; an in-flight request or
  component can no longer delay process exit indefinitely.
- `credential`'s circuit-breaker read no longer consumes half-open probe slots
  (`try_acquire` was being used as a predicate).

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
