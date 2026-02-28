# Reliability

## SLO Targets

- availability:
  - >= 99.95% successful acquire for healthy registered resources (platform target).
- latency:
  - acquire p95 <= 50ms for warm pools in normal load.
  - acquire p99 <= 250ms excluding external system cold-start.
- error budget:
  - retryable acquire failures budgeted separately from fatal configuration failures.

## Failure Modes

- dependency outage:
  - external DB/API/queue unavailable causing create/validate failures.
- timeout/backpressure:
  - `PoolExhausted` and timeout spikes under burst workloads.
- partial degradation:
  - resource degraded/unhealthy while system continues with reduced capacity.
- data corruption:
  - stale/invalid pooled instances if recycle/validation policies are weak.

## Resilience Strategies

- retry policy:
  - retries are caller-owned; resource crate classifies retryability.
- circuit breaking:
  - integrate with `resilience` at orchestration layer; `CircuitBreakerOpen` reserved for interoperable semantics.
- fallback behavior:
  - fail fast for unregistered/non-compatible scope resources.
  - degrade workloads on non-critical resources where supported by caller logic.
- graceful degradation:
  - quarantine unhealthy resources and route around optional dependencies where possible.

## Operational Runbook

- alert conditions:
  - rapid increase in `PoolExhausted`, timeout rate, or unhealthy/quarantine counts.
- dashboards:
  - acquire latency percentiles, active/idle pool stats, create/cleanup failures, waiter depth.
- incident triage steps:
  1. identify impacted resource IDs and tenants/workflows.
  2. inspect health/quarantine events and recent config reloads.
  3. verify external dependency health and credential freshness.
  4. apply mitigation (scale, policy adjustment, temporary disable, rollback).

## Capacity Planning

- load profile assumptions:
  - bursty action execution with mixed short- and long-lived resource usage.
- scaling constraints:
  - max pool size per resource, external service limits, and worker concurrency.
  - maintain headroom to avoid synchronized create storms.
