# Reliability

## SLO Targets

- **Availability:** N/A (library; no service)
- **Latency:** `SystemInfo::get()` <1µs; `memory::current()` <1ms; `process::list()` <50ms (100 processes)
- **Error budget:** Graceful degradation on permission/parse errors; no panics in normal paths

## Failure Modes

- **Dependency outage:** sysinfo/region failures mapped to `SystemError`
- **Timeout/backpressure:** No built-in timeouts; caller responsibility
- **Partial degradation:** Missing process/network/disk info returns empty/default when feature disabled
- **Data corruption:** Parse errors return `SystemParseError`; no silent corruption

## Resilience Strategies

- **Retry policy:** Caller-defined; no retries inside crate
- **Circuit breaking:** N/A
- **Fallback behavior:** `SystemInfo::get()` without sysinfo returns minimal info (cores from `available_parallelism`)
- **Graceful degradation:** Feature-gated modules return empty/default when disabled

## Operational Runbook

- **Alert conditions:** N/A (library)
- **Dashboards:** N/A
- **Incident triage:** If consumers see `PermissionDenied`, check process privileges; `FeatureNotSupported` indicates missing feature flag

## Capacity Planning

- **Load profile assumptions:** Polling every 1–10s typical; not for high-frequency (>10 Hz) monitoring
- **Scaling constraints:** Single-process; no distributed state
