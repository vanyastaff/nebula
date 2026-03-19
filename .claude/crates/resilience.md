# nebula-resilience
Fault-tolerance patterns — circuit breaker, retry, bulkhead, rate limiter, timeout — with compile-time const-generic configuration.

## Invariants
- `benches/compose.rs` is an API contract documenting the `LayerStack`/`ResilienceLayer` compose API. Any signature change must be reflected there. Run `cargo bench --no-run -p nebula-resilience` to verify it still compiles.
- `cargo fmt` on this crate causes formatting churn. Edit `src/` only; run `cargo fmt` workspace-wide only immediately before commit.

## Key Decisions
- Const generics for compile-time config validation: `CircuitBreakerConfig::<FAILURE_THRESHOLD, RESET_TIMEOUT_MS>` — wrong values are caught at compile time.
- `LayerStack` / `LayerBuilder` for composing multiple patterns: `LayerBuilder::new().circuit_breaker(...).retry(...).build()`.
- `Gate` = graceful-shutdown barrier — wrap all in-flight operations, then close the gate before shutting down.
- Typestate pattern on `CircuitBreaker` — state transitions tracked at the type level.

## Traps
- **RetryFailure wrapping**: inside `CircuitBreaker::execute(|| async { retry.execute_resilient(...).await })`, the inner failure type is `RetryFailure<E>`. Unwrap with `.map_err(|f| f.error)`.
- `RetryStrategy` vs `RetryConfig`: `RetryConfig` holds the parameters; `RetryStrategy` is the executor. Build strategy from config via `RetryStrategy::new(config)?`.
- `StandardCircuitBreaker` / `FastCircuitBreaker` / `SlowCircuitBreaker` are type aliases with preset const params — use them instead of specifying raw const generics.

## Relations
- No nebula deps. Used by nebula-resource, nebula-credential, nebula-engine for external call resilience.
