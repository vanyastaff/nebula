# Reliability

## SLO Targets

- availability:
  - high success rate for allocations under configured healthy limits.
- latency:
  - stable low-latency allocation/reuse on warm paths.
- error budget:
  - retryable exhaustion under spikes is acceptable; corruption-class failures are near-zero tolerance.

## Failure Modes

- dependency outage:
  - degraded monitoring behavior if system-level memory info is unavailable.
- timeout/backpressure:
  - pool/budget exhaustion under burst demand.
- partial degradation:
  - pressure escalation forces reduced allocation modes.
- data corruption:
  - invariant violations in unsafe internals or incorrect reset lifecycle.

## Resilience Strategies

- retry policy:
  - caller-owned; rely on retryability from `MemoryError`.
- circuit breaking:
  - handled in resilience/runtime layer, not hidden in memory core.
- fallback behavior:
  - switch from advanced paths to simpler alloc strategies when needed.
- graceful degradation:
  - restrict large allocations and prioritize critical operations under pressure.

## Operational Runbook

- alert conditions:
  - spikes in `PoolExhausted`, `BudgetExceeded`, pressure critical transitions.
- dashboards:
  - allocated bytes, failure rates, pressure level distribution, cache/pool hit ratios.
- incident triage steps:
  1. identify failing module and workload segment.
  2. inspect pressure and budget metrics.
  3. verify config sizing and recent deployment changes.
  4. apply mitigation: scale, throttle, reconfigure limits, or rollback.

## Capacity Planning

- load profile assumptions:
  - mixed bursty workflow executions with repeated short-lived allocations.
- scaling constraints:
  - per-process memory bounds, pool capacities, and host memory pressure.
