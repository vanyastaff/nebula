# Reliability

## SLO Targets

- availability:
  - tenant decision service path meets platform control-plane availability target.
- latency:
  - tenant context+quota decision stays within runtime admission budget.
- error budget:
  - transient backend failures budgeted; cross-tenant safety failures are zero-tolerance.

## Failure Modes

- dependency outage:
  - policy/config/storage backend unavailable.
- timeout/backpressure:
  - high QPS tenant lookups and quota updates cause contention.
- partial degradation:
  - stale policy cache or delayed usage reconciliation.
- data corruption:
  - inconsistent tenant metadata or quota counters.

## Resilience Strategies

- retry policy:
  - retry only transient backend/lock contention errors.
- circuit breaking:
  - handled in runtime/resilience wrappers around tenant backend calls.
- fallback behavior:
  - fail closed for identity/isolation checks.
  - conservative throttling when quota state is uncertain.
- graceful degradation:
  - prioritize safety and isolation over throughput in degraded mode.

## Operational Runbook

- alert conditions:
  - spikes in invalid-tenant and quota-violation rates
  - backend latency/errors affecting tenant decisions
- dashboards:
  - decision latency, denial reasons, quota utilization, per-tenant saturation
- incident triage steps:
  1. identify affected tenants and failure class.
  2. validate policy/config backend health.
  3. inspect recent tenant policy changes.
  4. apply mitigation (rollback policy, throttle, failover backend).

## Capacity Planning

- load profile assumptions:
  - high-cardinality tenant access with bursty execution admission.
- scaling constraints:
  - policy cache size, backend RTT, and quota write contention.
