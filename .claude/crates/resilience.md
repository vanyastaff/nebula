# nebula-resilience
Fault-tolerance patterns: circuit breaker, retry, bulkhead, rate limiter, timeout, load shedding.

## Invariants
- `benches/compose.rs` is an API contract for `ResiliencePipeline`. Run `cargo bench --no-run -p nebula-resilience` after signature changes.
- **`CallError<E>` is the sole error type** — no `ResilienceError`. All patterns use it.
- `CallError<E>` implements `Classify` when `E: Classify` — delegates for `Operation`/`RetriesExhausted`, fixed categories for pattern variants.
- **`retry()` and `retry_with()` require `E: Classify`** — auto-skips non-retryable errors, respects `retry_hint().after` as backoff delay floor. `retry_if()` overrides classification.
- **Pipeline uses `retry_with_inner()`** — `pub(crate)` version without Classify bound for `Option<E>` wrapping. Both share a single `retry_loop()` implementation.
- **Only `nebula-error` dep** — otherwise standalone. No `futures`/`dashmap`/`serde_json`.
- `RateLimiter` trait methods return `impl Future + Send`. Pipeline uses `RateLimitCheck` closure or `rate_limiter_from()` helper.
- **`total_budget`** is wall-clock based — tracks elapsed time including operation execution, not just sleep time.
- **`HedgeExecutor::new()` returns `Result`** — validates `HedgeConfig` at construction.
- **All patterns use `.call()` method** — unified verb across CB, bulkhead, timeout, hedge, fallback, rate limiter, pipeline, cancellation.
- **`CircuitBreaker::try_acquire()`** — not `can_execute`. Returns `Result`, not `bool`.
- **`Outcome` NOT re-exported at root** — access via `circuit_breaker::Outcome`.
- **`ResilienceEvent::kind()`** — method, not freestanding function.

## Traps
- **Successes decrement failure count** in Closed state ("leaky bucket" forgiveness).
- **CB `call()` is cancel-safe**: `ProbeGuard` RAII releases probe slots on drop.
- **CB callbacks fire OUTSIDE the lock** — prevents deadlock if callback reads CB state.
- **`FunctionFallback` erases `Operation(E)`**: closure receives `CallError<()>`.
- **`TokenBucket::burst_size`** is `AtomicUsize` — updated in-place by `update_burst()`. `AdaptiveRateLimiter` keeps burst in sync with rate.
- **`AdaptiveRateLimiter` counters** are lock-free atomics. Write lock only taken for rate adjustment when stats window elapses.
- **Field name**: `max_half_open_operations` (not `half_open_max_ops`).

## Relations
- Depends on: nebula-error. Used by nebula-resource for pool resilience, nebula-credential for refresh circuit breaker.

<!-- reviewed: 2026-03-31 — naming audit + file reorganization: types.rs→error.rs, policy_source.rs+signals.rs→policy.rs -->
