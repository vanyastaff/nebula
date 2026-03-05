# Test Strategy

## Test Pyramid

- **Unit:** pattern logic (retry backoff, circuit state transitions, bulkhead acquire/release, rate limiter); error classification; policy validation.
- **Integration:** pattern composition (retry + circuit, timeout + bulkhead, full chain); manager execute with multiple services; cancellation propagation.
- **Contract:** policy serialization round-trip; `Retryable` trait compatibility with `ActionError`.
- **End-to-end:** via engine/runtime; resilience as part of workflow execution (future).

## Critical Invariants

- Circuit breaker opens after `failure_threshold` failures; transitions to half-open after `reset_timeout`; closes on success.
- Retry stops after `max_attempts`; backoff respects `base_delay`, `max_delay`, jitter.
- Bulkhead never exceeds `max_concurrency`; queued ops respect `queue_size` and `timeout`.
- Timeout aborts operation; `timeout_with_original_error` preserves cause.
- `ResilienceError::is_retryable()` matches `ErrorClass`; `retry_after` present for rate limit and circuit open when applicable.

## Scenario Matrix

- **Happy path:** success on first attempt; circuit closed; bulkhead acquire/release; rate limit permit.
- **Retry path:** transient failure → retry → success; retry limit exceeded → `RetryLimitExceeded`.
- **Cancellation path:** `tokio::select!` with `Cancelled`; hedge cancellation; timeout cancellation.
- **Timeout path:** operation exceeds timeout → `Timeout`; nested timeouts (outer aborts first).
- **Upgrade/migration path:** policy schema change; config migration; compatibility tests.

## Tooling

- **Property testing:** optional; proptest for policy config validity.
- **Fuzzing:** optional; serde policy deserialization.
- **Benchmarks:** `cargo bench -p nebula-resilience` — circuit_breaker, rate_limiter, manager, retry, compose, timeout, bulkhead, fallback, hedge, observability.
- **CI quality gates:** `cargo test -p nebula-resilience`; `cargo clippy`; `cargo fmt --check`.

## Coverage Map

| Layer | Scope | Key suites |
|---|---|---|
| Unit | Core pattern correctness, policy validation, error taxonomy | `src/**/tests` (`circuit_breaker`, `retry`, `bulkhead`, `rate_limiter`, `timeout`, `manager`, `policy`) |
| Integration | Cross-pattern behavior and concurrency/fault semantics | `integration_pattern_composition.rs`, `integration_fault_injection.rs`, `integration_concurrent_access.rs`, `integration_bulkhead_fairness.rs`, `integration_retry_storm_guard.rs`, `integration_fallback_fault_injection.rs`, `integration_hedge_stress.rs`, `integration_observability_storm.rs` |
| Benchmark | Hot-path latency and contention profiles with Criterion | `benches/{manager,rate_limiter,circuit_breaker,retry,compose,timeout,bulkhead,fallback,hedge,observability}.rs` |

## Regression Gates

- Required checks for Phase 8 regression control:
	- `cargo test -p nebula-resilience`
	- `cargo clippy -p nebula-resilience -- -D warnings`
	- `cargo bench -p nebula-resilience --bench fallback`
	- `cargo bench -p nebula-resilience --bench hedge`
	- `cargo bench -p nebula-resilience --bench observability`
- Gate thresholds and interpretation policy are defined in `PERFORMANCE_BUDGET.md`.

## Exit Criteria

- **Coverage goals:** critical paths (circuit, retry, bulkhead, timeout) covered; integration tests for composition.
- **Flaky test budget:** time-dependent tests use `tokio::time::pause()`/`advance()` where possible; avoid sleep in hot paths.
- **Performance regression thresholds:** benchmark baselines; no significant regression on manager hot path or pattern overhead.
