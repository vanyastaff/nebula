# Reliability

## SLO Targets

- **Availability:** N/A — resilience is a library, not a service. No uptime target.
- **Latency:** pattern overhead (circuit check, retry delay, bulkhead acquire) should be sub-millisecond in hot path; timeout enforcement adds configured delay.
- **Error budget:** N/A. Resilience reduces downstream error impact; circuit breaker and retry limits bound failure propagation.

## Failure Modes

- **Dependency outage:** when external service (HTTP, DB, queue) fails, resilience applies retry/circuit/fallback. Circuit opens after threshold; retries stop after limit.
- **Timeout/backpressure:** timeout aborts long-running ops; bulkhead limits concurrency; rate limiter throttles. `BulkheadFull`, `RateLimitExceeded` returned to caller.
- **Partial degradation:** circuit open → fail-fast; bulkhead full → reject; rate limit → throttle. Fallback/hedge provide alternative paths when configured.
- **Data corruption:** resilience does not persist state; policy config corruption caught by validation. No data integrity concerns.

## Resilience Strategies

- **Retry policy:** exponential/fixed/linear backoff; jitter; `max_attempts`; `Retryable` trait for error classification.
- **Circuit breaking:** failure threshold, reset timeout, half-open probe; fail-fast when open.
- **Fallback behavior:** `FallbackStrategy`, `ValueFallback`; primary failure triggers fallback path.
- **Graceful degradation:** hedge (parallel slow path); bulkhead isolation; rate limiting to protect downstream.

## Operational Runbook

- **Alert conditions:** high circuit-open rate; retry exhaustion; bulkhead rejections; rate limit hits. Emit via observability hooks.
- **Dashboards:** circuit state, retry attempts, bulkhead utilization, rate limit usage. Use `ObservabilityHook`, `MetricsHook`, spans.
- **Incident triage:** 1) check circuit state and failure threshold; 2) verify downstream service health; 3) adjust policy (e.g., increase timeout, retry limit) or fix root cause; 4) reset circuit if safe.

## Capacity Planning

- **Load profile assumptions:** configurable per service; default timeouts (30s), retries (3), circuit threshold (5). Tune for HTTP (10s), DB (5s), queue (30s).
- **Scaling constraints:** circuit breaker and rate limiter are per-instance; bulkhead limits concurrency. For horizontal scaling, consider per-node limits and global rate limits (future).
