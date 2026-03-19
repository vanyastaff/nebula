# nebula-resilience
Fault-tolerance patterns — circuit breaker, retry, bulkhead, rate limiter, timeout, hedge, load shedding — with `CallError<E>`, `MetricsSink`, and plain-struct configuration.

## Invariants
- `benches/compose.rs` is an API contract documenting the `ResiliencePipeline` API. Run `cargo bench --no-run -p nebula-resilience` to verify it still compiles.
- **`CallError<E>` is the sole error type** — no `ResilienceError` anywhere. All patterns, fallback, and cancellation use `CallError<E>`.
- All patterns accept an optional `MetricsSink` via `.with_sink(sink)` for observability. Default is `NoopSink`.
- `CircuitBreakerConfig` derives `serde::Serialize, serde::Deserialize` for cross-crate use.
- `CircuitBreaker::can_execute::<E>()` is public for manual circuit-state checking in integrations.
- **Zero internal nebula deps** — fully standalone. Uses `tracing` directly.

## Key Decisions
- **Plain-struct config** for all patterns: `CircuitBreakerConfig { failure_threshold: 3, .. }` with runtime `.validate()` returning `ConfigError`.
- **Unified `CallError<E>`**: wraps user error `E` in `Operation(E)` or pattern-specific variants (`CircuitOpen`, `BulkheadFull`, `RateLimited`, `Timeout`, `RetriesExhausted`, `Cancelled`, `LoadShed`). `CallErrorKind` for fieldless dispatch. `.is_retriable()` returns true for `Timeout`, `RateLimited`, `BulkheadFull`.
- **`FallbackStrategy<T, E>`**: generic over both value and error type. Uses `CallError<E>` directly. `PriorityFallback` dispatches by `CallErrorKind`.
- **`CancellationContext::execute()`**: returns `Result<T, CallError<E>>`. `CancellableFuture::Output` is `Result<F::Output, CallError<()>>`.
- **Simplified Retry API**: `retry(n, f)` / `retry_with(config, f)`. `BackoffConfig::delay_for()` is public.
- **`ResiliencePipeline<E>`**: supports 6 patterns — `timeout`, `retry`, `circuit_breaker`, `bulkhead`, `rate_limiter`, `load_shed`. Recommended order: `load_shed → rate_limiter → timeout → retry → circuit_breaker → bulkhead`.
- **`TokenBucket::new()` returns `Result`** — no silent clamping of capacity/refill_rate.
- **`timeout()` unified** — single function handling `Result<T, E>` futures. Old `timeout_with_original_error` removed.
- **CB tripping logic**: opens when `failures >= failure_threshold AND total >= min_operations`. `failure_rate_threshold` removed entirely.
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
- `failure_rate_threshold` field from `CircuitBreakerConfig`

## Traps
- **Successes decrement failure count**: `record_outcome(Success)` does `failures.saturating_sub(1)` when Closed ("leaky bucket" forgiveness).
- **`TokenBucket::burst_size`** caps refill (not `capacity`). If `with_burst()` is used, the burst cap differs from the initial capacity.
- **`CallError::map_operation`**: use `.map_operation(f)` to transform the inner error type `E`.
- **`RateLimiter` trait is not dyn-compatible** (uses `async_fn_in_trait`). Pipeline uses `RateLimitCheck` closure instead.

## Relations
- Zero internal nebula deps (fully standalone). Used by nebula-resource for pool resilience (`CircuitBreaker`, `Gate`, `CircuitBreakerConfig`).
- `CircuitState` is `Copy`.
- `PipelineBuilder<E>` implements `Default`.

<!-- reviewed: 2026-03-19 (major API review: error unification, dead code removal, CB fixes, pipeline expansion) -->
