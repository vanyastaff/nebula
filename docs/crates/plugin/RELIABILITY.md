# Reliability

## SLO Targets

- **Availability:** Plugin operations are synchronous and in-process; no external dependencies. Registry available when process is up.
- **Latency:** Registry lookup < 1µs (in-memory HashMap).
- **Error budget:** N/A; plugin is bootstrap/discovery layer, not request path.

## Failure Modes

- **Dependency outage:** N/A; plugin has no async I/O in core path. Dynamic loader may fail to open library — returns `PluginLoadError`.
- **Timeout/backpressure:** N/A; no async operations.
- **Partial degradation:** Version not found → `PluginError::VersionNotFound`; caller can fall back to another version.
- **Data corruption:** Registry is in-memory; no persistence. Process restart clears state.

## Resilience Strategies

- **Retry policy:** Not applicable; plugin ops are sync and deterministic.
- **Circuit breaking:** N/A.
- **Fallback behavior:** Caller can use `get_plugin(None)` for latest when specific version fails.
- **Graceful degradation:** Registry can be populated incrementally; missing plugin → `NotFound`.

## Operational Runbook

- **Alert conditions:** Dynamic load failures (if loader used); high rate of NotFound (may indicate misconfiguration).
- **Dashboards:** Plugin count; load failures (if loader enabled).
- **Incident triage:** Check registry population; verify plugin paths (dynamic loading).

## Capacity Planning

- **Load profile assumptions:** Registry populated at startup; lookups during workflow compilation/execution. Low QPS.
- **Scaling constraints:** In-memory registry; size bounded by number of plugins (typically < 1000).
