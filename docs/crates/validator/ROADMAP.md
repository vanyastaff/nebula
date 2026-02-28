# Roadmap

## Status Snapshot

- completed:
  - Phase 1 baseline docs/contracts are in place.
  - config integration category compatibility baseline is pinned via fixtures.
- in progress:
  - Phase 2 governance automation (registry/process hardening).
- next focus:
  - enforce machine-readable compatibility checks in CI without slowing local dev loop.

## Phase 1: Contract and Docs Baseline (Completed)

- deliverables:
  - full template-aligned docs with cross-crate contracts
  - canonical API examples aligned to current implementation
  - explicit compatibility policy draft
- risks:
  - hidden behavior assumptions in downstream crates
- exit criteria:
  - docs accepted as single source of truth for current API
  - no stale naming mismatch in public docs

## Phase 2: Compatibility and Governance (In Progress)

- deliverables:
  - machine-readable error code/category registry
  - compatibility tests for error codes/field paths
  - deprecation and migration policy enforcement
  - governance checks requiring migration mapping for behavior-significant changes
- risks:
  - legacy consumers relying on undocumented error details
- exit criteria:
  - backward compatibility CI checks in place
  - migration-map checks running for release candidates

## Phase 3: Performance and Capacity Hardening (Planned)

- deliverables:
  - benchmark budgets for common validator/combinator chains
  - cache strategy guidance (`cached`) for expensive checks
  - allocation-focused profiling for heavy failure paths
- risks:
  - regressions in deeply nested combinator usage
- exit criteria:
  - benchmark thresholds enforced in CI

## Phase 4: Ecosystem and DX (Planned)

- deliverables:
  - advanced patterns for workflow/plugin/sdk consumers
  - optional schema/policy layer evaluation
  - macro debugging and authoring guidance
- risks:
  - over-expansion of API surface
- exit criteria:
  - clear stable core vs optional extension boundaries

## Execution Plan (Next 2 Iterations)

1. Governance pass:
  - formalize canonical registry artifact location and review rule.
  - add CI job that fails on non-additive minor-contract edits without migration mapping.
2. Contract pass:
  - expand cross-crate fixture assertions for config/api adapters.
  - freeze serializer envelope examples used by operator tooling.
3. Performance pass:
  - define bench profiles: quick PR profile vs release profile.
  - document hard threshold policy and exception process.

## Metrics of Readiness

- correctness:
  - zero known semantic drift in contract tests
- latency:
  - p95 validation latency budget for representative pipelines
- throughput:
  - benchmarked sustained validations/sec for hot paths
- stability:
  - no flaky contract tests
- operability:
  - actionable error telemetry with stable code mapping
