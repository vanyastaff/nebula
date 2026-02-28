# Reliability

## SLO Targets

- **Availability:** Metrics recording must not affect execution availability. Export endpoint: best-effort; scrape failures do not impact application.
- **Latency:** Recording < 1µs; scrape response < 100ms.
- **Error budget:** Export failures do not consume execution error budget.

## Failure Modes

| Mode | Impact | Mitigation |
|------|--------|------------|
| Export endpoint down | Scrape fails; no metrics in Prometheus | Retry; alert on scrape failure |
| OTLP push failure | Metrics not in collector | Retry with backoff; optional |
| High cardinality | Memory growth; scrape slow | Bounded labels; histogram buckets |
| Registry lock contention | Recording latency | Minimize lock scope; atomics |

## Resilience Strategies

- **Retry policy:** OTLP push retries with exponential backoff; configurable.
- **Circuit breaking:** Optional for OTLP; stop push after repeated failures.
- **Fallback behavior:** Recording always succeeds; export best-effort.
- **Graceful degradation:** Export fails → metrics buffered or dropped; execution continues.

## Operational Runbook

- **Alert conditions:** Scrape failure; high metric cardinality; export errors.
- **Dashboards:** Execution count; error rate; latency percentiles; resource usage.
- **Incident triage:** Metrics gaps do not block execution; fix export or scrape.

## Capacity Planning

- **Load profile assumptions:** 1k–10k metrics/sec; scrape every 15–60s.
- **Scaling constraints:** Single process metrics; no distributed aggregation in MVP.
