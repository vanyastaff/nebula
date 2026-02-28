# Reliability

## SLO Targets

- **Availability:** Telemetry is best-effort; execution does not depend on it. Target: emit/record never blocks execution.
- **Latency:** Emit < 1µs p99; metric record < 1µs p99. No synchronous I/O.
- **Error budget:** N/A; telemetry failures do not affect execution SLO.

## Failure Modes

| Mode | Impact | Mitigation |
|------|--------|------------|
| Broadcast channel full | Lagging subscribers; oldest events dropped | Increase capacity; subscriber catches up or skips |
| No subscribers | Events dropped | Acceptable; fire-and-forget design |
| Histogram lock poisoned | Panic on observe | Rare; indicates programmer error |
| MetricsRegistry lock poisoned | Panic on counter/gauge/histogram | Rare; same as above |

## Resilience Strategies

- **Retry policy:** N/A; no retries (emit is fire-and-forget).
- **Circuit breaking:** N/A; no external calls.
- **Fallback behavior:** NoopTelemetry for testing; always safe.
- **Graceful degradation:** If subscriber lags, events dropped; execution continues.

## Operational Runbook

- **Alert conditions:** Subscriber lag (if monitored); metric cardinality growth.
- **Dashboards:** Execution count; node duration percentiles; error rate (when exporter available).
- **Incident triage:** Telemetry issues do not block execution; investigate only if observability gap affects ops.

## Capacity Planning

- **Load profile assumptions:** 1k–10k events/sec for typical workflow throughput.
- **Scaling constraints:** Broadcast capacity (default 64–128); Histogram memory (Phase 2: bounded). Single process; no distributed telemetry in MVP.
