# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - stabilize sandbox port docs and runtime integration points.
  - define capability model schema and violation error contracts.
  - add policy guardrails for backend selection.
- risks:
  - mismatch between action metadata and enforceable capability semantics.
- exit criteria:
  - contract tests pass for in-process path and cancellation/error propagation.

## Phase 2: Runtime Hardening

- deliverables:
  - structured violation/audit events and observability dashboards.
  - enforce capability checks in `SandboxedContext` access paths.
  - improve policy-driven fallback behavior on backend issues.
- risks:
  - false positives in capability checks causing execution failures.
- exit criteria:
  - deterministic violation handling with low false-positive rate.

## Phase 3: Scale and Performance

- deliverables:
  - benchmark sandbox overhead per backend.
  - optimize hot-path context and serialization boundaries.
  - establish SLOs for sandbox decision + execution overhead.
- risks:
  - added policy checks increasing action latency.
- exit criteria:
  - overhead within accepted runtime budget for trusted workloads.

## Phase 4: Ecosystem and DX

- deliverables:
  - ship full-isolation backend (`wasm` and/or `process`).
  - provide action authoring guidelines for capability declarations.
  - add migration and compatibility tooling for backend transitions.
- risks:
  - backend parity issues and operational complexity.
- exit criteria:
  - production-ready path for untrusted/community actions.

## Metrics of Readiness

- correctness:
  - policy/capability invariants covered by contract tests.
- latency:
  - sandbox overhead remains within target execution budget.
- throughput:
  - stable action throughput under expected concurrency.
- stability:
  - low flaky rate in backend integration tests.
- operability:
  - actionable telemetry for violations and backend health.
