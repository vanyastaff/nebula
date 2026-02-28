# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - create `crates/tenant` with context model, identity validation, and baseline policy API.
  - implement fail-closed ingress checks and explicit tenant errors.
  - document and test cross-crate contract boundaries.
- risks:
  - hidden tenant assumptions in existing runtime/api paths.
- exit criteria:
  - end-to-end validated tenant context propagation for critical workflows.

## Phase 2: Runtime Hardening

- deliverables:
  - quota accounting with concurrency-safe updates.
  - admission checkpoints in runtime/execution/resource-heavy operations.
  - audit trail and observability integration.
- risks:
  - quota race conditions and noisy false positives.
- exit criteria:
  - deterministic quota behavior under stress and retry scenarios.

## Phase 3: Scale and Performance

- deliverables:
  - optimize high-cardinality tenant workloads.
  - cache hot policy paths with bounded staleness guarantees.
  - tune telemetry cardinality and aggregation strategy.
- risks:
  - policy cache inconsistency and hot-tenant bottlenecks.
- exit criteria:
  - stable latency and throughput across representative tenant distributions.

## Phase 4: Ecosystem and DX

- deliverables:
  - partition strategy tooling and migration assistants.
  - tenant policy templates for common deployment tiers.
  - comprehensive operator runbooks.
- risks:
  - operational complexity during strategy changes.
- exit criteria:
  - safe rollout playbook validated in staging.

## Metrics of Readiness

- correctness:
  - no cross-tenant policy bypass in contract tests.
- latency:
  - tenant decision path within runtime SLO budget.
- throughput:
  - sustained policy checks under peak concurrency.
- stability:
  - no critical flaky tests in identity/quota paths.
- operability:
  - actionable tenant audit and quota telemetry.
