# Reliability

## SLO Targets

- **Availability:** Log init is one-shot; runtime logging is best-effort (no blocking on writer)
- **Latency:** Hot path (emit) should add &lt;10µs p99 in default config
- **Error budget:** Init failures are fatal to app startup; runtime log write failures depend on writer

## Failure Modes

- **Dependency outage:** OTLP/Sentry unreachable — telemetry degrades; logging continues
- **Timeout/backpressure:** File writer blocking — consider non-blocking writer; document drop policy
- **Partial degradation:** One hook panics — isolated; others continue
- **Data corruption:** Config parse failure — init returns error; no partial state

## Resilience Strategies

- **Retry policy:** Init is not retried by crate; caller may retry
- **Circuit breaking:** N/A for init; hook policy supports bounded execution budgets with over-budget diagnostics
- **Fallback behavior:** `auto_init` falls back to dev/prod preset if env unset
- **Graceful degradation:** Telemetry features disabled → logging still works

## Operational Runbook

- **Alert conditions:** Log init failure (app won't start); hook error rate (if metrics exposed)
- **Dashboards:** Standard tracing/metrics dashboards from OTLP/Prometheus
- **Incident triage:** Check config/env (see API env contract table); verify writer permissions; inspect hook registry

## Capacity Planning

- **Load profile assumptions:** High event rate in workflow execution; bursts during workflow runs
- **Scaling constraints:** Global subscriber; single registry; bounded hook policy mitigates slow-hook latency amplification
