# nebula-resilience
Fault-tolerance patterns: circuit breaker, retry, bulkhead, rate limiter, timeout, load shedding.

## Quick mental model
- `ResiliencePipeline` composes layers in add-order (first added = outermost).
- Recommended order: `load_shed -> rate_limiter -> timeout -> retry -> circuit_breaker -> bulkhead`.
- `CallError<E>` is the single cross-pattern error contract; all adapters should preserve this shape.
- `retry_with()` is Classify-aware; pipeline retry path uses `retry_with_inner()` and classifier plumbing.
- Circuit breaker probe lifecycle is cancellation-sensitive and guarded by RAII (`ProbeGuard`).
- Rate limiting has 4 built-in algorithms plus optional `governor` implementation.

## Where to look first
- Pipeline composition and layer semantics: `src/pipeline.rs`
- Retry/backoff/jitter and budget handling: `src/retry.rs`
- Circuit state machine and probe handling: `src/circuit_breaker.rs`
- Concurrency gating/queueing: `src/bulkhead.rs`, `src/gate.rs`
- Rate algorithms and adaptive tuning: `src/rate_limiter.rs`
- Error contract and remapping helpers: `src/error.rs`
- Metrics/events surface: `src/sink.rs`

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
- **Retry budget check uses `checked_add`** — huge backoff durations cannot panic on `Duration` overflow.
- **`HedgeExecutor::new()` returns `Result`** — validates `HedgeConfig`.
- **`AdaptiveHedgeExecutor::with_max_samples(n)`** — configures latency tracker capacity (default 1000). Returns `Err` if n=0.
- **`AdaptiveHedgeExecutor` uses `parking_lot::RwLock`** — not `tokio::sync::RwLock`: both `record()` and `percentile()` are sync, no `.await` under lock.
- **`LatencyTracker` uses `Vec<(u64, u32)>` histogram** — sorted by nanos, no BTreeMap, no heap allocs after warmup. `ring: VecDeque<u64>` stores nanos (not Duration).
- **CB `SlidingWindow` uses two `Box<[u8]>` arrays** — `failure_ring` and `slow_ring` separate for SIMD vectorization of sum loops. No `OutcomeEntry` struct.
- **CB failure/slow rate checks use multiply form** — `failures >= threshold * total` instead of `failures/total >= threshold`, eliminating `divsd`.
- **`SlidingWindow` pre-allocates `VecDeque::with_capacity(max_requests)`** — no reallocs during warmup.
- **`SlidingWindow::acquire()` computes cutoff before lock** — `now.checked_sub(window_duration)` happens before `mutex.lock()`, not inside `clean_old_requests_locked`.
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
- **`AdaptiveRateLimiter::reset()` is panic-free**: rebuilds TokenBucket with a safe fallback path (no `expect`).
- **`GovernorRateLimiter::new()` hardens non-finite rates** by clamping to a safe minimum before `from_secs_f64`.
- **Bulkhead wait queue guard uses `defused: bool`**, not `mem::forget`, for panic-safe counter handling.
- **Bulkhead queue timeout** returns `CallError::Timeout`, not `BulkheadFull`.

## Safety-critical hotspots
- `retry`: budget check must stay overflow-safe (`checked_add`) before comparing durations.
- `retry`: jitter must guard non-finite/negative float paths before `from_secs_f64`.
- `bulkhead`: queue wait counter must use defused RAII semantics, never `mem::forget`.
- `circuit_breaker`: half-open probe slots must be released on cancel/drop paths.
- `rate_limiter`: `f64 -> Duration` conversions must be capped/sanitized before conversion.
- `adaptive limiter reset`: keep panic-free reconstruction path (no `expect` in runtime code).

## Feature flags and what they change
- `governor`: enables `GovernorRateLimiter` (GCRA path in `src/rate_limiter.rs`).
- `humantime`: human-readable `Duration` serde support.
- `full`: enables both optional features.

## Fast validation commands
- Single crate (default features): `rtk cargo check -p nebula-resilience && rtk cargo nextest run -p nebula-resilience`
- Governor path: `rtk cargo nextest run -p nebula-resilience --features governor`
- Pre-PR workspace gate: `rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace`
- Targeted micro-benches: `cargo bench -p nebula-resilience --bench latency_tracker --features bench` and `--bench sliding_window_cb`

## Common change playbooks
- Touching `retry`:
	- Re-check `BackoffConfig` edge values and jitter finite handling.
	- Validate `total_budget` semantics (elapsed op time + next delay).
- Touching `circuit_breaker`:
	- Verify half-open probe accounting on success/error/cancel paths.
	- Confirm state-change callbacks still run outside locks.
- Touching `pipeline`:
	- Re-run `benches/compose.rs` contract compile.
	- Verify retry + classifier + fallback error mapping still preserves `CallError<E>`.
- Touching `rate_limiter`:
	- Validate constructor bounds and conversion safety.
	- Run tests with and without `governor` feature.

## Module structure
```
error.rs        — CallError, ConfigError, CallErrorKind, CallResult
policy.rs       — PolicySource, LoadSignal, ConstantLoad (was policy_source.rs + signals.rs)
sink.rs         — MetricsSink, ResilienceEvent, ResilienceEventKind, RecordingSink
pipeline.rs     — ResiliencePipeline, PipelineBuilder
+ pattern modules: bulkhead, circuit_breaker, fallback, hedge, load_shed, rate_limiter, retry, timeout
+ infra: cancellation, clock, gate
```

## When to use this crate
Any outgoing call (HTTP, DB, external service, plugin execution) should go through `ResiliencePipeline` or individual patterns. Specifically:
- **Retry + timeout** — any call that can transiently fail (network, rate limits)
- **CircuitBreaker** — protect against cascading failures from a degraded downstream
- **Bulkhead** — limit concurrency to prevent resource exhaustion (e.g., connection pools)
- **RateLimiter** — enforce throughput limits (API quotas, token budgets)
- **Fallback** — graceful degradation (cached values, defaults, chain of alternatives)
- **Gate** — cooperative shutdown barrier for request handlers

Prefer `ResiliencePipeline` for composing multiple patterns — it handles layer ordering warnings, CB probe guards, and retry error classification automatically.

## Relations
- Depends on: nebula-error. Used by nebula-resource (pool resilience), nebula-credential (refresh CB).

<!-- reviewed: 2026-04-01 — ASM perf fixes: LatencyTracker BTreeMap→Vec, CB SlidingWindow u8 split, divsd elimination, cutoff-before-lock, trip_open_from_half_open helper -->
