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

Helpers on `CallError<E>`:

- `cancelled()`, `cancelled_with(reason)`
- `fallback_failed()`, `fallback_failed_with(reason)`
- `rate_limited()`, `rate_limited_after(duration)`
- `is_retryable()`
- `is_cancellation()`
- `into_operation()`, `operation()`
- `map_operation()`, `flat_map_inner()`
- `retry_after()`
- `kind() -> CallErrorKind`

Related types:

- `CallErrorKind`
- `CallResult<T, E>`
- `ConfigError { field, message }`

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
- `timeout(duration)`
- `retry(config)`
- `circuit_breaker(Arc<CircuitBreaker>)`
- `bulkhead(Arc<Bulkhead>)`
- `rate_limiter(check)`
- `rate_limiter_from(Arc<impl RateLimiter>)`
- `load_shed(predicate)`
- `build()`

`ResiliencePipeline<E>` methods:

- `builder()`
- `call(factory)`
- `call_with_fallback(factory, &dyn FallbackStrategy<T, E>)`

Notes:

- First added step is the outermost wrapper.
- Recommended order: `load_shed -> rate_limiter -> timeout -> retry -> circuit_breaker -> bulkhead`.
- Pipeline retry preserves `RetryConfig` backoff, jitter, classifier / predicate, callback, sink, and total budget.
- Pipeline retry does not apply `retry_hint().after` as a delay floor; use standalone `retry_with()` when you need per-error retry-after handling.
- `with_sink()` records pipeline-level `TimeoutElapsed`, `RateLimitExceeded`, and `LoadShed` events.

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
- `call_with_classifier(factory, classifier)`
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
- `acquire()`
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

---

## Timeout and Load Shedding

Root re-exports:

- `timeout(duration, future)`
- `TimeoutExecutor`
- `load_shed(predicate, factory)`
- `load_shed_with_sink(predicate, factory, sink)`

Notable methods:

- `TimeoutExecutor::new(duration)`
- `TimeoutExecutor::with_sink(sink)`
- `TimeoutExecutor::call(future)`

Notes:

- `timeout_with_sink()` exists in the `timeout` module and is used by `TimeoutExecutor`.
- `load_shed_with_sink()` emits `ResilienceEvent::LoadShed`.

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

---

## Hedging

Root re-exports:

- `HedgeConfig`
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

- Hedged calls are intentionally not cancel-safe; dropping the returned future does not automatically stop already-spawned hedge tasks.

---

## Cancellation and Gate

Root re-exports:

- `CancellationContext`
- `CancellableFuture<F>`
- `CancellationExt`
- `Gate`
- `GateGuard`
- `GateClosed`

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

---

## Observability and Policy

Root re-exports:

- `MetricsSink`
- `NoopSink`
- `RecordingSink`
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

`RecordingSink` helpers:

- `new()`
- `events()`
- `count(kind)`
- `has_state_change(state)`
