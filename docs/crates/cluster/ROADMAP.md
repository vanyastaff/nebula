# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - create `crates/cluster` skeleton and core contracts.
  - membership model + safe join/leave APIs.
  - basic placement API with deterministic behavior.
- risks:
  - unclear ownership boundaries with existing runtime logic.
- exit criteria:
  - compile-time and integration contract checks pass with runtime.

## Phase 2: Runtime Hardening

- deliverables:
  - failover detection and idempotent rescheduling.
  - durable control-plane state integration with storage.
  - observability hooks for cluster lifecycle events.
- risks:
  - failover race conditions and duplicate assignments.
- exit criteria:
  - failure-injection tests show deterministic recovery.

## Phase 3: Scale and Performance

- deliverables:
  - optimize scheduler for high-cardinality workflow distributions.
  - strategy tuning and fairness guarantees.
  - benchmark cluster decision latency under load.
- risks:
  - scheduling hotspots and leader bottlenecks.
- exit criteria:
  - placement latency and throughput within target SLO budgets.

## Phase 4: Ecosystem and DX

- deliverables:
  - operator APIs/CLI for rebalance and maintenance.
  - autoscaling policy framework and staged rollout.
  - runbooks for incident triage and cluster operations.
- risks:
  - operational complexity and misconfiguration risks.
- exit criteria:
  - production readiness checklist validated in staging.

## Metrics of Readiness

- correctness:
  - no split-brain placement ownership in test scenarios.
- latency:
  - scheduler/control-plane latency within budget.
- throughput:
  - stable assignment throughput at target concurrency.
- stability:
  - robust failover with low incident regression rate.
- operability:
  - actionable metrics and audit trails for cluster operations.
