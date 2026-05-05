# nebula-resilience — API Reference

This document tracks the current public surface of `nebula-resilience`.
For exhaustive item-level details, prefer the generated rustdoc from `src/lib.rs`.

---

## Core Types

### `CallError<E>`

Returned by every resilience pattern and by `ResiliencePipeline`.

Variants:

- `Operation(E)`
- `CircuitOpen`
- `BulkheadFull`
- `Timeout(Duration)`
- `RetriesExhausted { attempts, last }`
- `Cancelled { reason }`
- `LoadShed`
- `RateLimited { retry_after }`
- `FallbackFailed { reason }`
- `FallbackFailedWithContext { primary, fallback }`

Helpers on `CallError<E>`:

- `cancelled()`, `cancelled_with(reason)`
- `fallback_failed()`, `fallback_failed_with(reason)`
- `fallback_failed_with_context(primary, fallback)`
- `rate_limited()`, `rate_limited_after(duration)`
- `is_retryable()`
- `is_cancellation()`
- `into_operation()`, `operation()`
- `map_operation()`, `flat_map_inner()`
- `retry_after()`
- `fallback_context()`
- `kind() -> CallErrorKind`

Related types:

- `CallErrorKind`
- `CallResult<T, E>`
- `ConfigError { field, message }`
- `PolicyContext`
- `Deadline`

`PolicyContext` methods:

- `empty()`
- `from_cancellation(CancellationContext)`
- `with_timeout(duration)`
- `with_cancellation(CancellationContext)`
- `with_deadline(Deadline)`
- `with_scope(PolicyScope)`
- `child()`
- `cancellation()`
- `deadline()`
- `scope()`
- `is_cancelled()`

`Deadline` methods:

- `after(duration)`
- `from_start(instant, duration)`
- `budget()`
- `elapsed()`
- `remaining()`
- `remaining_or_timeout()`
- `timeout(future)`
- `sleep(duration)`

---

## Pipeline

Root re-exports:

- `ResiliencePipeline<E>`
- `PipelineBuilder<E>`
- `RateLimitCheck`
- `LoadShedPredicate`

`PipelineBuilder<E>` methods:

- `new()`
- `classifier(Arc<dyn ErrorClassifier<E>>)`
- `classify_errors()` when `E: nebula_error::Classify`
- `with_sink(sink)`
- `scope(PolicyScope)`
- `timeout(duration)`
- `retry(config)`
- `circuit_breaker(Arc<CircuitBreaker>)`
- `bulkhead(Arc<Bulkhead>)`
- `rate_limiter(check)`
- `rate_limiter_from(Arc<impl RateLimiter>)`
- `rate_limiter_erased(Arc<dyn ErasedRateLimiter>)`
- `load_shed(predicate)`
- `build_recommended_order()`
- `build_checked() -> Result<ResiliencePipeline<E>, ConfigError>`
- `build()`

`ResiliencePipeline<E>` methods:

- `builder()`
- `call(factory)`
- `call_with_context(&CancellationContext, factory)`
- `call_with_fallback(factory, &dyn FallbackStrategy<T, E>)`
- `call_with_context_and_fallback(&CancellationContext, factory, &dyn FallbackStrategy<T, E>)`
- `call_with_policy_context(&PolicyContext, factory)`
- `call_with_policy_context_and_fallback(&PolicyContext, factory, &dyn FallbackStrategy<T, E>)`

Notes:

- First added step is the outermost wrapper.
- Recommended order: `load_shed -> rate_limiter -> timeout -> retry -> circuit_breaker -> bulkhead`.
- `build_checked()` rejects order inversions with `ConfigError`; use it for schema/config-driven policy assembly where warnings are insufficient.
- Pipeline retry preserves `RetryConfig` backoff, jitter, classifier / predicate, callback, sink, and total budget.
- Operation errors are permanent by default unless `retry_if`, `with_classifier`, `classifier`, or `classify_errors()` marks them retryable.
- Pipeline retry preserves `retry_hint().after` as a delay floor for classified operation errors and rate-limit rejections.
- `call_with_context()` lets cancellation interrupt retry sleep, timeout wrappers, bulkhead acquisition, rate-limit checks, and the operation.
- `call_with_context_and_fallback()` additionally prevents cancellation from being reported as fallback recovery; cancellation wins before and during the fallback future.
- `call_with_policy_context()` and `call_with_policy_context_and_fallback()` also apply a context deadline to the whole call and use context scope for `PipelineCompleted` when set.
- `with_sink()` records pipeline-level `TimeoutElapsed`, `RateLimitExceeded`, `LoadShed`, and fallback lifecycle events.
- Every pipeline call emits `PipelineCompleted { scope, outcome }`; fallback recovery is represented as `PipelineOutcome::FallbackSucceeded` instead of a plain success.
- `FallbackStrategy::fallback(error)` is the safe entry point and checks
  `should_fallback()` before invoking strategy recovery. Built-in `ChainFallback`
  and `PriorityFallback` use that safe entry point for nested strategies, so
  cancellation and overload-style policy errors are not recovered by later fallbacks
  unless a custom strategy explicitly opts in.
- `FunctionFallback` preserves the primary failure in `FallbackFailedWithContext`
  when the fallback closure itself fails. Custom strategy recovery still receives
  the primary error by value, so universal primary-error preservation for every
  fallback implementation remains future API work.

---

## Retry

Root re-exports:

- `retry(n, factory)`
- `retry_with(config, factory)`
- `RetryConfig<E>`
- `BackoffConfig`
- `JitterConfig`

`RetryConfig<E>` builder methods:

- `new(max_attempts) -> Result<Self, ConfigError>`
- `backoff(BackoffConfig)`
- `jitter(JitterConfig)`
- `total_budget(Duration)`
- `with_classifier(Arc<dyn ErrorClassifier<E>>)`
- `retry_if(predicate)`
- `on_retry(callback)`
- `with_sink(sink)`

`BackoffConfig` variants:

- `Fixed(Duration)`
- `Linear { base, max }`
- `Exponential { base, multiplier, max }`
- `Fibonacci { base, max }`
- `Custom(SmallVec<[Duration; 8]>)`

`JitterConfig` variants:

- `None`
- `Full { factor, seed }`

---

## Circuit Breaker

Root re-exports:

- `CircuitBreaker`
- `CircuitBreakerConfig`

`CircuitBreakerConfig` fields:

- `failure_threshold`
- `reset_timeout`
- `max_half_open_operations`
- `half_open_success_threshold`
- `min_operations`
- `count_timeouts_as_failures`
- `break_duration_multiplier`
- `max_break_duration`
- `slow_call_threshold`
- `slow_call_rate_threshold`
- `sliding_window_size`
- `failure_rate_threshold`

Key `CircuitBreaker` methods:

- `new(config) -> Result<Self, ConfigError>`
- `with_sink(sink)`
- `with_clock(clock)`
- `call(factory)`
- `call_with_classifier(classifier, factory)`
- `call_with_policy_context(context, factory)`
- `call_with_classifier_and_policy_context(classifier, context, factory)`
- `try_acquire()`
- `record_outcome(outcome)`
- `circuit_state()`
- `stats()`
- `force_open()`
- `force_close()`

Module-level public type:

- `circuit_breaker::Outcome`

---

## Bulkhead

Root re-exports:

- `Bulkhead`
- `BulkheadConfig`

`BulkheadConfig` fields:

- `max_concurrency`
- `queue_size`
- `timeout`

Key methods:

- `Bulkhead::new(config) -> Result<Self, ConfigError>`
- `with_sink(sink)`
- `call(factory)`
- `call_with_policy_context(context, factory)`
- `acquire()`
- `acquire_with_policy_context(context)`
- `stats()`
- `active_operations()`
- `available_permits()`
- `is_at_capacity()`
- `max_concurrency()`

Queueing:

- `queue_size` may be `0` (no wait queue: if no permit is free, return `BulkheadFull` immediately).
- When `queue_size` is at least `1`, that many callers may wait for a permit; further callers get `BulkheadFull`.

---

## Rate Limiting

Root re-exports:

- `RateLimiter`
- `ErasedRateLimiter`
- `TokenBucket`
- `LeakyBucket`
- `SlidingWindow`
- `AdaptiveRateLimiter`

Feature-gated module export:

- `rate_limiter::GovernorRateLimiter` with feature `governor`

Constructors:

- `TokenBucket::new(capacity, refill_rate)`
- `LeakyBucket::new(capacity, leak_rate)`
- `SlidingWindow::new(window_duration, max_requests)`
- `AdaptiveRateLimiter::new(initial_rate, min_rate, max_rate)`
- `GovernorRateLimiter::new(rate_per_second, burst_capacity)` when enabled

Notable extras:

- `TokenBucket::with_burst()`, `update_rate()`, `update_burst()`
- `AdaptiveRateLimiter::record_success()`, `record_error()`
- `RateLimiter::acquire_with_policy_context()`, `call_with_policy_context()`
- `ErasedRateLimiter::acquire_boxed()`, `acquire_with_policy_context_boxed()`, `current_rate_boxed()`, `reset_boxed()`

Use `ErasedRateLimiter` for tenant/resource registries that need heterogeneous
limiters as `Arc<dyn ErasedRateLimiter>`. Use `RateLimiter` directly when the
concrete limiter type is known at compile time.

---

## Timeout and Load Shedding

Root re-exports:

- `timeout(duration, future)`
- `timeout_with_policy_context(context, duration, future)`
- `timeout_with_policy_context_and_sink(context, duration, future, sink)`
- `TimeoutExecutor`
- `load_shed(predicate, factory)`
- `load_shed_with_sink(predicate, factory, sink)`
- `load_shed_with_policy_context(context, predicate, factory)`
- `load_shed_with_policy_context_and_sink(context, predicate, factory, sink)`

Notable methods:

- `TimeoutExecutor::try_new(duration)`
- `TimeoutExecutor::new(duration)`
- `TimeoutExecutor::with_sink(sink)`
- `TimeoutExecutor::with_shared_sink(Arc<dyn MetricsSink>)`
- `TimeoutExecutor::call(future)`
- `TimeoutExecutor::call_with_policy_context(context, future)`

Notes:

- `timeout_with_sink()` exists in the `timeout` module and is used by `TimeoutExecutor`.
- `timeout_with_policy_context*()` uses the earlier of the local timeout and the
  context deadline; context cancellation wins without polling the future.
- `TimeoutExecutor::try_new()` rejects zero durations. `timeout(Duration::ZERO, ...)`
  is an immediate timeout and does not poll the protected future.
- `load_shed_with_sink()` emits `ResilienceEvent::LoadShed`.
- `load_shed_with_policy_context*()` checks context cancellation/deadline before
  evaluating the shed predicate and bounds the in-flight operation by the context.

---

## Fallbacks

Public module: `nebula_resilience::fallback`

Root re-exports:

- `FallbackStrategy<T, E>`
- `ValueFallback<T>`

Module types:

- `FunctionFallback<T, F, Fut>`
- `CacheFallback<T>`
- `ChainFallback<T, E>`
- `PriorityFallback<T, E>`
- `FallbackOperation<T, E>`

`FallbackOperation<T, E>` methods:

- `new(strategy)`
- `with_sink(sink)`
- `with_shared_sink(Arc<dyn MetricsSink>)`
- `call(factory)`
- `call_with_policy_context(context, factory)`

Default fallback behavior:

- Custom fallback implementations normally implement strategy recovery and leave
  `fallback()` as the safe wrapper.
- `should_fallback()` returns true for operation failures, retry exhaustion, timeout,
  and open circuit.
- Cancellation, load shedding, rate limiting, bulkhead rejection, and fallback failure
  are not recovered by default; custom strategies may opt into those cases.
- `ChainFallback` keeps passing the latest error through the chain, but each nested
  strategy's own `should_fallback()` is respected. A fallback-side cancellation or
  contextual fallback failure is therefore not converted into a later fallback success
  by default.
- `PriorityFallback` dispatches by `CallErrorKind`, then still respects the selected
  strategy's `should_fallback()` before recovery.
- Standalone `FallbackOperation` emits `FallbackAttempted`, `FallbackSucceeded`, and
  `FallbackFailed` when a sink is attached. Cancellation/deadline errors that are not
  recovered do not emit fallback lifecycle events.

---

## Hedging

Root re-exports:

- `HedgeConfig`
- `HedgeSafety`
- `HedgeExecutor`

Public module: `nebula_resilience::hedge`

Additional module type:

- `AdaptiveHedgeExecutor`

Notable methods:

- `HedgeExecutor::new(config) -> Result<Self, ConfigError>`
- `HedgeExecutor::with_sink(sink)`
- `HedgeExecutor::call(factory)`
- `AdaptiveHedgeExecutor::new(config) -> Result<Self, ConfigError>`
- `AdaptiveHedgeExecutor::with_target_percentile(percentile)`
- `AdaptiveHedgeExecutor::with_max_samples(max_samples)`
- `AdaptiveHedgeExecutor::with_sink(sink)`
- `AdaptiveHedgeExecutor::call(factory)`

Important caveat:

- `HedgeConfig::default()` sends no duplicate requests (`max_hedges = 0`).
- `max_hedges > 0` requires `duplicate_safety = HedgeSafety::Idempotent`.
- Dropping a `HedgeExecutor::call` future aborts tasks owned by that call, but side effects already sent to a remote service are not reversible.

---

## Cancellation and Gate

Root re-exports:

- `CancellationContext`
- `CancellableFuture<F>`
- `CancellationExt`
- `Gate`
- `GateGuard`
- `GateClosed`
- `GateCloseTimeout`

Notable methods:

- `CancellationContext::new()`
- `CancellationContext::with_reason(reason)`
- `CancellationContext::child()`
- `CancellationContext::cancel()`
- `CancellationContext::call(factory)`
- `CancellationContext::call_with_timeout(factory, timeout)`
- `Gate::new()`
- `Gate::enter()`
- `Gate::close()`
- `Gate::close_with_timeout(timeout)`
- `Gate::active_count()`
- `Gate::is_closed()`

---

## Observability and Policy

Root re-exports:

- `MetricsSink`
- `NoopSink`
- `RecordingSink`
- `PolicyScope`
- `ScopeValue`
- `PipelineOutcome`
- `ResilienceEvent`
- `ResilienceEventKind`
- `CircuitState`
- `PolicySource<C>`
- `LoadSignal`
- `ConstantLoad`

`ResilienceEvent` variants:

- `CircuitStateChanged { from, to }`
- `RetryAttempt { attempt, will_retry }`
- `BulkheadRejected`
- `TimeoutElapsed { duration }`
- `HedgeFired { hedge_number }`
- `RateLimitExceeded`
- `LoadShed`
- `FallbackAttempted { primary_error }`
- `FallbackSucceeded { primary_error }`
- `FallbackFailed { primary_error }`
- `PipelineCompleted { scope, outcome }`

`RecordingSink` helpers:

- `new()`
- `events()`
- `count(kind)`
- `has_state_change(state)`
