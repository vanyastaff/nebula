# Reliability

## SLO Targets

- **Availability:** Determined by API/worker deployment; engine is library. Target: execute_workflow completes or returns error; no silent loss of execution.
- **Latency:** Bounded by workflow size and node execution latency; engine overhead should be small (scheduling, param resolution).
- **Error budget:** Transient failures (runtime, action) may be retried per resilience policy; fatal errors (planning, validation) are not retried.

## Failure Modes

- **Runtime/action failure:** Node fails; engine maps to NodeFailed, may retry; event emitted. State persisted so execution can resume or be inspected.
- **Dependency outage:** Expression engine, plugin registry, or resource manager unavailable — engine fails fast or degrades (e.g. skip optional resource). EventBus lag does not block execution.
- **Timeout/backpressure:** Execution budget or timeout; optional admission control (P-002) rejects or queues when overloaded.
- **Data corruption:** State store failure; engine should not corrupt state; persist transitions atomically where possible.

## Resilience Strategies

- **Retry:** Per resilience/action contract; engine interprets ActionResult (Retry, Fatal) and applies policy.
- **Circuit breaking:** Not in engine; optional in runtime or API.
- **Fallback:** EventBus best-effort; no fallback for state store (durability required for resume).
- **Graceful degradation:** Under pressure, reject new executions (admission) rather than slow or fail in-flight.

## Operational Runbook

- **Alert conditions:** High failure rate, execution stuck, state store errors; metrics on execution count and duration.
- **Dashboards:** Execution status, node completion rate, error breakdown by EngineError variant.
- **Incident triage:** Check state store, runtime, and event bus health; verify workflow definition and action keys.

## Capacity Planning

- **Load profile:** Concurrency limited by runtime/worker pool and optional admission; engine scales with CPU for scheduling and param resolution.
- **Scaling constraints:** Single-process engine; horizontal scaling via multiple workers each with own engine instance.
