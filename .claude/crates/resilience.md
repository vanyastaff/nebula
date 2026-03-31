# nebula-resilience
Fault-tolerance patterns: circuit breaker, retry, bulkhead, rate limiter, timeout, load shedding.

## Invariants
- `benches/compose.rs` is an API contract for `ResiliencePipeline`. Run `cargo bench --no-run -p nebula-resilience` after signature changes.
- **`CallError<E>` is the sole error type** — `#[non_exhaustive]`, includes `FallbackFailed` variant.
- `CallError<E>` implements `Classify` when `E: Classify` — delegates for `Operation`/`RetriesExhausted`, fixed categories for pattern variants.
- **`CallError::flat_map_inner()`** — DRY helper for variant remapping. Used by pipeline retry and FunctionFallback.
- **`retry()` takes `NonZeroU32`** — no panic on zero. `retry_with()` requires `E: Classify`.
- **Pipeline uses `retry_with_inner()`** — `pub(crate)` version without Classify bound. Both share `retry_loop()`.
- **Only `nebula-error` dep** — otherwise standalone.
- `RateLimiter` trait: `call()` has default impl (acquire + operation). Override only for custom behavior (e.g. AdaptiveRateLimiter).
- **`total_budget`** is wall-clock based — tracks elapsed time including operation execution.
- **`HedgeExecutor::new()` returns `Result`** — validates `HedgeConfig`.
- **All patterns use `.call()` method** — unified verb across all executors.
- **`CircuitBreaker::try_acquire()`** — returns `Result`, not `bool`.
- **`Outcome` NOT re-exported at root** — access via `circuit_breaker::Outcome`.
- **`ResilienceEvent::kind()` returns `ResilienceEventKind`** — typed enum, not `&str`.
- **All 5 pub enums are `#[non_exhaustive]`**: `CallError`, `BackoffConfig`, `ResilienceEvent`, `Outcome`, `CircuitState`.
- **All public types implement `Debug`** — manual impls for types with closures/Arc<dyn>.
- **Config types have serde**: `CircuitBreakerConfig`, `BulkheadConfig`, `HedgeConfig`, `BackoffConfig`, `JitterConfig`.

## Traps
- **Successes decrement failure count** in Closed state ("leaky bucket" forgiveness).
- **`count_timeouts_as_failures=false`** — timeouts completely ignored: not counted as failures, not in `total`, not toward `min_operations`. Probe slot IS released in HalfOpen.
- **`ProbeGuard` uses `defused: bool` flag** (not `mem::forget`). Panic-safe: Drop only records Cancelled when not defused.
- **CB callbacks fire OUTSIDE the lock** — prevents deadlock if callback reads CB state.
- **`FunctionFallback` erases `Operation(E)` → `FallbackFailed`** (not `Cancelled`). Closure receives `CallError<()>`.
- **`TokenBucket::burst_size`** is `AtomicUsize` — updated in-place by `update_burst()`. `AdaptiveRateLimiter` keeps burst in sync with rate.
- **`AdaptiveRateLimiter` counters** are lock-free atomics. Write lock only taken for rate adjustment.
- **Field name**: `max_half_open_operations` (not `half_open_max_ops`).
- **`ChainFallback::then()`** (not `add`). **`PriorityFallback`** uses `Vec` (not `HashMap`).
- **Seeded jitter** mixes seed with attempt number — each retry gets different jitter.
- **`MockClock::now()`** includes real elapsed time (Instant limitation on stable Rust). Use large advances in tests.
- **Bulkhead queue timeout** returns `CallError::Timeout`, not `BulkheadFull`.

## Module structure
```
error.rs        — CallError, ConfigError, CallErrorKind, CallResult
policy.rs       — PolicySource, LoadSignal, ConstantLoad (was policy_source.rs + signals.rs)
sink.rs         — MetricsSink, ResilienceEvent, ResilienceEventKind, RecordingSink
pipeline.rs     — ResiliencePipeline, PipelineBuilder
+ pattern modules: bulkhead, circuit_breaker, fallback, hedge, load_shed, rate_limiter, retry, timeout
+ infra: cancellation, clock, gate
```

## Relations
- Depends on: nebula-error. Used by nebula-resource (pool resilience), nebula-credential (refresh CB).

<!-- reviewed: 2026-03-31 — full audit: bugs, naming, API guidelines, design patterns, 10-dimension code review -->
