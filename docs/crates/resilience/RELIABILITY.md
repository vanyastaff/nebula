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

## Fail-Open / Fail-Closed Defaults

The crate uses **fail-closed by default** for protective controls. Explicit graceful-degradation patterns (`fallback`, `hedge`) are opt-in.

| Pattern | Default | Behavior |
|---|---|---|
| `timeout` | **Fail-closed** | On deadline exceed returns `ResilienceError::Timeout`; operation result is not accepted. |
| `bulkhead` | **Fail-closed** | At capacity/queue overflow returns `BulkheadFull`; acquire timeout returns `Timeout`. |
| `rate_limiter` | **Fail-closed** | On limit hit returns `RateLimitExceeded` with retry hint where available. |
| `circuit_breaker` | **Fail-closed** | In open state rejects with `CircuitBreakerOpen` until half-open probe window. |
| `retry` | **Conditional fail-closed** | Retries only retryable errors; terminal/no-budget paths end with original error or `RetryLimitExceeded`. |
| `fallback` | **Fail-open (opt-in)** | If configured and eligible, returns degraded value; if fallback chain fails -> `FallbackFailed` (fail-closed). |
| `hedge` | **Fail-open (opt-in)** | Returns first successful replica; if none succeeds, returns failure/timeout. |

### Operational Contract

- Without `fallback`/`hedge`, resilience never masks failures: errors propagate upstream.
- `fallback`/`hedge` are explicit business decisions to trade strict correctness for availability/latency.
- Invalid policy updates are treated as no-op (existing runtime state remains active), preventing accidental fail-open behavior from bad config.

### Override Guidance

- Prefer fail-closed for write paths and side-effecting operations.
- Allow fail-open only for read/derived data where stale/default responses are acceptable.
- Record degraded-path usage with observability hooks and alert on sustained activation.

## Operational Runbook

- **Alert conditions:** high circuit-open rate; retry exhaustion; bulkhead rejections; rate limit hits. Emit via observability hooks.
- **Dashboards:** circuit state, retry attempts, bulkhead utilization, rate limit usage. Use `ObservabilityHook`, `MetricsHook`, spans.
- **Incident triage:** 1) check circuit state and failure threshold; 2) verify downstream service health; 3) adjust policy (e.g., increase timeout, retry limit) or fix root cause; 4) reset circuit if safe.

## Capacity Planning

- **Load profile assumptions:** configurable per service; default timeouts (30s), retries (3), circuit threshold (5). Tune for HTTP (10s), DB (5s), queue (30s).
- **Scaling constraints:** circuit breaker and rate limiter are per-instance; bulkhead limits concurrency. For horizontal scaling, consider per-node limits and global rate limits (future).
