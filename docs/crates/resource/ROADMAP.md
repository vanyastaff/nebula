# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - finalize and publish `resource` cross-crate contracts (`INTERACTIONS`, `API`, `MIGRATION`).
  - formalize error handling guidance (`retryable`, fatal, validation).
  - lock scope invariants and deny-by-default behavior.
- risks:
  - hidden assumptions in action/runtime integration paths.
- exit criteria:
  - contract docs map 1:1 with implementation.
  - no unresolved scope or error taxonomy ambiguities.

## Phase 2: Runtime Hardening

- deliverables:
  - strengthen shutdown and reload behavior tests under in-flight load.
  - improve health-to-quarantine propagation observability.
  - add operational guardrails for invalid config reload attempts.
- risks:
  - regressions in pool swap and long-tail create/cleanup failures.
- exit criteria:
  - deterministic shutdown in stress tests.
  - no leaked permit/instance in CI race scenarios.

## Phase 3: Scale and Performance

- deliverables:
  - benchmark-driven tuning for acquire latency and pool contention.
  - back-pressure policy profiles (`fail-fast`, bounded wait, adaptive) if accepted.
  - high-cardinality metrics hygiene for multi-tenant workloads.
- risks:
  - policy complexity and incorrect defaults under burst traffic.
- exit criteria:
  - p95 acquire latency and exhaustion rates within SLO targets.
  - no perf regressions versus current throughput baseline.

## Phase 4: Ecosystem and DX

- deliverables:
  - adapter crate guidance (`resource-postgres`, `resource-redis`, etc.).
  - typed key migration path (if approved) and developer ergonomics updates.
  - cookbook for runtime/action integration patterns.
- risks:
  - ecosystem fragmentation if adapter contracts diverge.
- exit criteria:
  - at least one reference adapter and end-to-end sample integration.

## Metrics of Readiness

- correctness:
  - invariant tests pass for scope, shutdown, health cascade, and dependency graph consistency.
- latency:
  - acquire p95/p99 tracked and bounded in benchmark profile.
- throughput:
  - sustained concurrent acquire/release without leak or starvation.
- stability:
  - no flaky critical-path tests in release CI.
- operability:
  - actionable events/metrics for triage and capacity planning.
