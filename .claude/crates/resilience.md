# nebula-resilience
Fault-tolerance patterns — circuit breaker, retry, bulkhead, rate limiter, timeout, hedge, load shedding — with `CallError<E>`, `MetricsSink`, and plain-struct configuration.

## Invariants
- `benches/compose.rs` is an API contract documenting the `ResiliencePipeline` API. Run `cargo bench --no-run -p nebula-resilience` to verify it still compiles.
- **`CallError<E>` is the sole error type** — no `ResilienceError` anywhere. All patterns, fallback, and cancellation use `CallError<E>`. Derives `Clone`, `PartialEq`, `Eq` when `E` does.
- All patterns accept an optional `MetricsSink` via `.with_sink(sink)` for observability. Default is `NoopSink`.
- `CircuitBreakerConfig` derives `serde::Serialize, serde::Deserialize` for cross-crate use.
- `CircuitBreaker::can_execute::<E>()` is public for manual circuit-state checking in integrations.
- **Zero internal nebula deps** — fully standalone. Uses `tracing` directly. No `futures`/`dashmap`/`serde_json` in main deps.

## Key Decisions
- **Plain-struct config** for all patterns: `CircuitBreakerConfig { failure_threshold: 3, .. }` with runtime `.validate()` returning `ConfigError`.
- **Unified `CallError<E>`**: wraps user error `E` in `Operation(E)` or pattern-specific variants (`CircuitOpen`, `BulkheadFull`, `RateLimited`, `Timeout`, `RetriesExhausted`, `Cancelled`, `LoadShed`). `CallErrorKind` for fieldless dispatch. `.is_retriable()` returns true for `Timeout`, `RateLimited`, `BulkheadFull`.
- **`FallbackStrategy<T, E>`**: generic over both value and error type. Uses `CallError<E>` directly. `PriorityFallback` dispatches by `CallErrorKind`.
- **`CancellationContext::execute()`**: returns `Result<T, CallError<E>>`. `CancellableFuture::Output` is `Result<F::Output, CallError<()>>`.
- **Simplified Retry API**: `retry(n, f)` / `retry_with(config, f)`. `BackoffConfig::delay_for()` is public. `JitterConfig::Full { factor, seed }` applies random jitter via `fastrand` (seed for deterministic tests).
- **Backoff strategies**: `Fixed`, `Linear`, `Exponential`, `Fibonacci { base, max }`, `Custom(Vec<Duration>)`.
- **Total delay budget**: `RetryConfig::total_budget(Duration)` stops retries if cumulative sleep time exceeds budget.
- **Retry notify**: `RetryConfig::on_retry(|err, delay, attempt|)` callback before each retry sleep.
- **Pipeline fallback**: `pipeline.call_with_fallback(f, &strategy)` wraps pipeline result with `FallbackStrategy`.
- **`ResiliencePipeline<E>`**: supports 6 patterns — `timeout`, `retry`, `circuit_breaker`, `bulkhead`, `rate_limiter`, `load_shed`. Recommended order: `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`.
- **All rate limiter constructors return `Result`** — `TokenBucket::new()`, `LeakyBucket::new()`, `SlidingWindow::new()` all validate and return `Result<Self, ConfigError>`.
- **`timeout()` unified** — single function handling `Result<T, E>` futures. Old `timeout_with_original_error` removed.
- **CB tripping logic**: count-based (`failures >= failure_threshold`) or rate-based (`failure_rate_threshold: Some(0.5)` + `sliding_window_size: 100`). Slow call rate also trips independently.
- **CB manual control**: `force_open()` / `force_close()` for ops. State callbacks via `.on_state_change(|from, to| { ... })`.
- **CB dynamic break duration**: `break_duration_multiplier` (default 1.0) + `max_break_duration` — reset timeout grows on consecutive opens.
- **CB slow call detection**: `slow_call_threshold: Some(Duration)` + `slow_call_rate_threshold: 0.5` — trips on degraded latency even without errors. `Outcome::SlowSuccess`/`SlowFailure`.
- **CB sliding window**: `sliding_window_size: 100` enables ring-buffer tracking. Old outcomes evict, rates recalculated per call.
- **RateLimited retry_after**: `CallError::RateLimited { retry_after: Some(Duration) }` with `.retry_after()` accessor.
- **`half_open_max_ops` enforced**: circuit rejects additional calls in `HalfOpen` once probe limit reached. Failure in HalfOpen immediately reopens.
- `Gate` = graceful-shutdown barrier.

## Removed (2026-03-19)
- `ResilienceError`, `ErrorClass`, `ErrorContext`, `CircuitBreakerOpenState` (error.rs)
- `ResilienceResult`, `ResultExt` (result.rs)
- `Retryable` trait (retryable.rs)
- `ObservabilityHooks`, `PatternEvent`, `Event<C>`, `LoggingHook`, `MetricsHook` (hooks.rs)
- `SpanGuard`, `PatternSpanGuard`, `create_span` (spans.rs)
- `MetricsCollector`, `MetricSnapshot` (metrics.rs)
- `ShutdownCoordinator` (cancellation.rs)
- `TimeoutPolicy`, `StrictPolicy`, `LenientPolicy`, `AdaptivePolicy` (timeout.rs)
- `AnyRateLimiter`, `AnyRateLimiterInner` (rate_limiter.rs)
- `ErrorCategory` (fallback.rs — replaced by `CallErrorKind`)
- `CircuitBreaker::with_config()` (redundant alias for `new()`)
- `failure_rate_threshold` field (v1) from `CircuitBreakerConfig` (re-added as `Option<f64>` in v2 with sliding window)
- `sliding_window` field (v1) from `CircuitBreakerConfig` (re-added as `sliding_window_size: u32` in v2)

## Traps
- **Successes decrement failure count**: `record_outcome(Success)` does `failures.saturating_sub(1)` when Closed ("leaky bucket" forgiveness).
- **CB `call()` is cancel-safe**: dropped futures release probe slots via `ProbeGuard` RAII. `record_outcome(Cancelled)` decrements `half_open_probes`. `call()` also measures duration for slow call classification via injected Clock.
- **CB callbacks fire OUTSIDE the lock**: `on_state_change` and `MetricsSink::record` are called after `drop(inner)` to prevent deadlock if the callback reads CB state.
- **Pipeline processes each step exactly once**: recursive `run_operation_with_shells` handles all step types. CB uses `can_execute()` + `ProbeGuard` + `record_outcome()`, Bulkhead uses `acquire()` — each called once per execution attempt.
- **`FunctionFallback` erases `Operation(E)`**: closure receives `CallError<()>`. If it returns `Operation(())`, that is mapped to `Cancelled` (not a panic).
- **`AdaptiveRateLimiter::new()` returns `Result`**: validates min/max rates against `TokenBucket` constraints.
- **`RateLimiter` trait is not dyn-compatible** (uses `async_fn_in_trait`). Pipeline uses `RateLimitCheck` closure instead.
- **`TokenBucket::burst_size`** caps refill (not `capacity`). If `with_burst()` is used, the burst cap differs from the initial capacity.

## Relations
- Zero internal nebula deps (fully standalone). Used by nebula-resource for pool resilience (`CircuitBreaker`, `Gate`, `CircuitBreakerConfig`).
- `CircuitState` is `Copy`.
- `PipelineBuilder<E>` implements `Default`.

<!-- reviewed: 2026-03-29 (pipeline fix, retry improvements, CB: manual control, callbacks, dynamic duration, slow calls, sliding window, RateLimited retry_after) -->
