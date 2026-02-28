# Reliability

## SLO Targets

- availability:
  - sandbox execution path matches runtime execution availability target.
- latency:
  - sandbox boundary overhead remains within runtime action budget.
- error budget:
  - transient backend failures budgeted; policy/violation correctness errors are near-zero tolerance.

## Failure Modes

- dependency outage:
  - runtime executor unavailability or driver initialization failure.
- timeout/backpressure:
  - long-running actions, cancellation delays, backend saturation.
- partial degradation:
  - fallback to in-process when full-isolation backend unavailable (policy-dependent).
- data corruption:
  - malformed serialization/metadata crossing sandbox boundary.

## Resilience Strategies

- retry policy:
  - runtime/resilience decides retries based on classified errors.
- circuit breaking:
  - per-action/backed circuit handling in resilience layer.
- fallback behavior:
  - explicit and policy-controlled backend fallback only.
- graceful degradation:
  - prefer safe deny over unsafe execution when policy integrity is uncertain.

## Operational Runbook

- alert conditions:
  - rising sandbox failures/violations/timeouts
  - backend init failures or policy mismatch events
- dashboards:
  - execution counts, latency distribution, violation counts, backend health
- incident triage steps:
  1. identify failing backend and action class.
  2. inspect policy mapping and recent config changes.
  3. evaluate fallback suitability.
  4. mitigate with policy rollback, backend restart, or action quarantine.

## Capacity Planning

- load profile assumptions:
  - bursty concurrent action execution with mixed trust levels.
- scaling constraints:
  - backend startup cost, serialization overhead, and host CPU/memory budgets.
