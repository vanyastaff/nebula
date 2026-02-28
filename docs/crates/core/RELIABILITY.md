# Reliability

## SLO Targets

- **Availability:** N/A — core is a library, not a service. No uptime target.
- **Latency:** ID creation/parse and scope operations should be sub-microsecond; no I/O.
- **Error budget:** N/A.

## Failure Modes

- **Dependency outage:** Core has no Nebula dependencies. External deps (serde, domain-key, chrono) are compile-time; no runtime outage.
- **Timeout/backpressure:** Core has no async I/O; no timeouts or backpressure.
- **Partial degradation:** N/A.
- **Data corruption:** Invalid serde input produces `CoreError::Deserialization`; consumers must handle. ID parse failures produce `UuidParseError` / `CoreError::InvalidInput`.

## Resilience Strategies

- **Retry policy:** Core does not retry. Consumers use `CoreError::is_retryable()` to decide.
- **Circuit breaking:** N/A.
- **Fallback behavior:** N/A.
- **Graceful degradation:** N/A.

## Operational Runbook

- **Alert conditions:** N/A — core is not deployed.
- **Dashboards:** N/A.
- **Incident triage:** If consumers hit core bugs (e.g., panic, wrong scope semantics), fix in core and release patch.

## Capacity Planning

- **Load profile:** Core types are used in hot paths (ID creation, context propagation). Zero allocation for IDs (Copy); minimal for scope/traits.
- **Scaling constraints:** None. Core is stateless and synchronous.
