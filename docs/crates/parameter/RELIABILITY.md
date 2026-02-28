# Reliability

## SLO Targets

- **Availability:** N/A (library; no service)
- **Latency:** Validation &lt;1ms for typical node configs (&lt;50 params, &lt;5 nesting levels)
- **Error budget:** All validation failures are deterministic; no retries

## Failure Modes

- **Dependency outage:** `nebula-validator` unavailable — validation would fail to compile; runtime dependency is in-process
- **Timeout/backpressure:** N/A; sync validation; no I/O
- **Partial degradation:** N/A; validation is all-or-nothing per call
- **Data corruption:** Malformed `ParameterValues` — type check catches; return `ParameterError`; no panic

## Resilience Strategies

- **Retry policy:** Validation is deterministic; retries not applicable (same input → same result)
- **Circuit breaking:** N/A
- **Fallback behavior:** N/A; no optional code paths
- **Graceful degradation:** Custom rules skipped if expression engine unavailable; other rules still run

## Operational Runbook

- **Alert conditions:** N/A (library)
- **Dashboards:** N/A
- **Incident triage:** If validation fails in production, check `ParameterError::code()` and `category()`; fix input or schema

## Capacity Planning

- **Load profile assumptions:** Validation per request/workflow execution; burst during workflow runs
- **Scaling constraints:** CPU-bound; large schemas (1000+ params, deep nesting) may need benchmarking
