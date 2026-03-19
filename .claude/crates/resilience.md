# nebula-resilience
Fault-tolerance patterns — circuit breaker, retry, bulkhead, rate limiter, timeout — with `CallError<E>`, `MetricsSink`, and plain-struct configuration.

## Invariants
- `benches/compose.rs` is an API contract documenting the `LayerStack`/`ResilienceLayer` compose API. Any signature change must be reflected there. Run `cargo bench --no-run -p nebula-resilience` to verify it still compiles.
- All patterns use `CallError<E>` as the unified error type — never raw `ResilienceError` from pattern public APIs.
- All patterns accept an optional `MetricsSink` via `.with_sink(sink)` for observability. Default is `NoopSink`.

## Key Decisions
- **Plain-struct config** for all patterns: `CircuitBreakerConfig { failure_threshold: 3, .. }` with runtime `.validate()` returning `ConfigError`. Old const-generic approach replaced.
- **Unified `CallError<E>`**: wraps user error `E` in `Operation(E)` or pattern-specific variants (`CircuitOpen`, `BulkheadFull`, `RateLimited`, `Timeout`, `RetriesExhausted`). `.is_retriable()` for retry decisions.
- **Simplified Retry API**: `retry(config, || async { ... })` and `retry_with(config, sink, || async { ... })`. Config via `RetryConfig`, `BackoffConfig`, `JitterConfig`. Old `RetryStrategy`/`RetryCondition`/`BackoffPolicy` hierarchy removed.
- **RateLimiter trait** returns `Result<(), CallError<()>>` from `acquire()` and `Result<T, CallError<E>>` from `execute()`.
- `LayerStack` / `LayerBuilder` for composing multiple patterns.
- `Gate` = graceful-shutdown barrier — wrap all in-flight operations, then close the gate before shutting down.
- `PolicySource<C>` + `LoadSignal` + `ConstantLoad` for adaptive policy configuration.

## Traps
- **compose.rs uses old `ResilienceError`**: `RateLimiterLayer` in `compose.rs` still maps `CallError` to `ResilienceError::RateLimitExceeded`. This is a known inconsistency pending compose API migration.
- **`CallError::map_err`**: use `.map_err(|e| e.map(f))` to transform the inner error type, not `.map_err(f)` directly.
- `StandardCircuitBreaker` / `FastCircuitBreaker` / `SlowCircuitBreaker` are type aliases with preset const params — use them instead of specifying raw const generics.

## Relations
- No nebula deps. Used by nebula-resource, nebula-credential, nebula-engine for external call resilience.

<!-- reviewed: 2026-03-18 -->
