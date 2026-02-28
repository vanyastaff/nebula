# Reliability

## SLO Targets

- availability:
  - cluster control-plane availability aligned with platform HA objectives.
- latency:
  - placement and failover decisions within runtime control-path budget.
- error budget:
  - transient network/backing-store failures budgeted; split-brain or ownership corruption is zero-tolerance.

## Failure Modes

- dependency outage:
  - storage/control backend unavailable.
- timeout/backpressure:
  - leader overload, slow consensus commits, operator command backlog.
- partial degradation:
  - reduced node set after failures with constrained scheduling capacity.
- data corruption:
  - inconsistent ownership/membership state.

## Resilience Strategies

- retry policy:
  - retry transient network/storage errors with bounded backoff.
- circuit breaking:
  - resilience layer around control-plane backend integrations.
- fallback behavior:
  - fail-safe no-new-placement mode when state integrity is uncertain.
- graceful degradation:
  - prioritize critical workflows and controlled admission during degraded state.

## Operational Runbook

- alert conditions:
  - leadership instability, placement failures, failover spikes.
- dashboards:
  - membership health, placement latency, failover counts, queue depth.
- incident triage steps:
  1. confirm leader and quorum health.
  2. inspect membership and storage consistency.
  3. review recent control operations and deployments.
  4. apply mitigation (rollback policy, rebalance freeze, controlled failover).

## Capacity Planning

- load profile assumptions:
  - bursty workflow placement requests with variable node health.
- scaling constraints:
  - leader throughput, consensus commit latency, and state backend performance.
