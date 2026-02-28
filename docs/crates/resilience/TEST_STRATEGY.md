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
- **Benchmarks:** `cargo bench -p nebula-resilience` — circuit_breaker, rate_limiter, manager, retry.
- **CI quality gates:** `cargo test -p nebula-resilience`; `cargo clippy`; `cargo fmt --check`.

## Exit Criteria

- **Coverage goals:** critical paths (circuit, retry, bulkhead, timeout) covered; integration tests for composition.
- **Flaky test budget:** time-dependent tests use `tokio::time::pause()`/`advance()` where possible; avoid sleep in hot paths.
- **Performance regression thresholds:** benchmark baselines; no significant regression on manager hot path or pattern overhead.
