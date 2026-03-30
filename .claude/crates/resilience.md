# nebula-resilience
Fault-tolerance patterns: circuit breaker, retry, bulkhead, rate limiter, timeout, load shedding.

## Invariants
- `benches/compose.rs` is an API contract for `ResiliencePipeline`. Run `cargo bench --no-run -p nebula-resilience` after signature changes.
- **`CallError<E>` is the sole error type** — no `ResilienceError`. All patterns use it.
- `CallError<E>` implements `Classify` when `E: Classify` — delegates for `Operation`/`RetriesExhausted`, fixed categories for pattern variants.
- **`retry()` and `retry_with()` require `E: Classify`** — auto-skips non-retryable errors, respects `retry_hint().after` as backoff floor. `retry_if()` overrides classification.
- **Pipeline uses `retry_with_inner()`** — `pub(crate)` version without Classify bound for `Option<E>` wrapping.
- **Only `nebula-error` dep** — otherwise standalone. No `futures`/`dashmap`/`serde_json`.
- `RateLimiter` trait is **not dyn-compatible** (async_fn_in_trait). Pipeline uses `RateLimitCheck` closure.

## Traps
- **Successes decrement failure count** in Closed state ("leaky bucket" forgiveness).
- **CB `call()` is cancel-safe**: `ProbeGuard` RAII releases probe slots on drop.
- **CB callbacks fire OUTSIDE the lock** — prevents deadlock if callback reads CB state.
- **`FunctionFallback` erases `Operation(E)`**: closure receives `CallError<()>`.
- **`TokenBucket::burst_size`** caps refill (not `capacity`). `with_burst()` differs from initial capacity.

## Relations
- Depends on: nebula-error. Used by nebula-resource for pool resilience.

<!-- reviewed: 2026-03-30 — dep cleanup: removed unused rand dev-dep -->
