# nebula-resilience
Fault-tolerance patterns — circuit breaker, retry, bulkhead, rate limiter, timeout, hedge, load shedding — with `CallError<E>`, `MetricsSink`, and plain-struct configuration.

## Invariants
- `benches/compose.rs` is an API contract documenting the `ResiliencePipeline` API. Run `cargo bench --no-run -p nebula-resilience` to verify it still compiles.
- All patterns use `CallError<E>` as the unified error type — no `ResilienceError` anywhere in pattern APIs (only in fallback/cancellation).
- All patterns accept an optional `MetricsSink` via `.with_sink(sink)` for observability. Default is `NoopSink`.
- `CircuitBreakerConfig` derives `serde::Serialize, serde::Deserialize` for cross-crate use.
- `CircuitBreaker::can_execute::<E>()` is public for manual circuit-state checking in integrations.
- **Zero internal nebula deps** — fully standalone. Uses `tracing` directly.

## Key Decisions
- **Plain-struct config** for all patterns: `CircuitBreakerConfig { failure_threshold: 3, .. }` with runtime `.validate()` returning `ConfigError`. Old const-generic approach removed.
- **Unified `CallError<E>`**: wraps user error `E` in `Operation(E)` or pattern-specific variants (`CircuitOpen`, `BulkheadFull`, `RateLimited`, `Timeout`, `RetriesExhausted`, `Cancelled`, `LoadShed`). `.is_cancellation()` for cancellation checks.
- **Simplified Retry API**: `retry(n, f)` / `retry_with(config, f)`. Config via `RetryConfig`, `BackoffConfig`, `JitterConfig`. `RetryConfig::new()` returns `Result<Self, ConfigError>` — validates `max_attempts >= 1`. Old `RetryStrategy`/`RetryCondition`/`BackoffPolicy` removed.
- **`ResiliencePipeline<E>`** replaces old `LayerBuilder`/`LayerStack`. Build via `ResiliencePipeline::builder().timeout(d).retry(cfg).circuit_breaker(cb).bulkhead(bh).build()`. Layers are outermost-first. Warns via `tracing::warn!` if timeout is inside retry.
- **`resilience::` module** for functional API: `resilience::retry`, `resilience::retry_with`, `resilience::with_timeout`, `resilience::load_shed`.
- **`load_shed(should_shed, f)`** functional pattern — returns `Err(CallError::LoadShed)` immediately when predicate fires.
- **`CircuitBreaker::circuit_state()`** is sync (returns `CircuitState`). **`record_outcome(Outcome)`** is sync.
- **CB tripping logic**: opens when `failures >= failure_threshold AND total >= min_operations`. `failure_rate_threshold` field exists but is NOT used by `record_outcome()`.
- `Gate` = graceful-shutdown barrier.
- `PolicySource<C>` + `LoadSignal` + `ConstantLoad` for adaptive policy configuration.
- **Dead code removed (2026-03-19)**: `ResilienceManager`, `PolicyBuilder`, `ResiliencePolicy`, typestate `PolicyBuilder<State>`, strategy markers, `core::categories`, `core::config`, `core::dynamic`, `core::traits`, `core::advanced` — all were unused downstream.

## Traps
- **`failure_rate_threshold` is ignored in CB tripping**: the circuit breaker opens based on absolute `failure_threshold` count, not rate. Doc comment on the field now says "Reserved — not used". Downstream tests that relied on rate-based tripping must be updated to use `failure_threshold`.
- **Successes decrement failure count**: `record_outcome(Success)` does `failures.saturating_sub(1)` when Closed, so alternating F/S patterns may never accumulate enough failures to trip the breaker.
- **`half_open_max_ops` is not enforced**: the config field exists but `can_execute()` in HalfOpen does not gate on it. All calls pass through unconditionally. Known issue pending fix.
- **`TokenBucket::burst_size`** now caps refill (not `capacity`). If `with_burst()` is used, the burst cap differs from the initial capacity.
- **`CallError::map_operation`**: use `.map_operation(f)` to transform the inner error type `E`.
- **`ResilienceError` still exists** — used by `fallback.rs` and `cancellation.rs`. Not part of pattern APIs but exported for those modules.

## Relations
- Zero internal nebula deps (fully standalone). Used by nebula-resource, nebula-credential, nebula-engine for external call resilience.

<!-- reviewed: 2026-03-19 -->
