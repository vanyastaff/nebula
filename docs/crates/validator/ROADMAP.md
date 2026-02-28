# Roadmap

## Phase 1: Contract and Docs Baseline

- deliverables:
  - full template-aligned docs with cross-crate contracts
  - canonical API examples aligned to current implementation
  - explicit compatibility policy draft
- risks:
  - hidden behavior assumptions in downstream crates
- exit criteria:
  - docs accepted as single source of truth for current API
  - no stale naming mismatch in public docs

## Phase 2: Compatibility and Governance

- deliverables:
  - machine-readable error code registry
  - compatibility tests for error codes/field paths
  - deprecation and migration policy enforcement
- risks:
  - legacy consumers relying on undocumented error details
- exit criteria:
  - backward compatibility CI checks in place

## Phase 3: Performance and Capacity Hardening

- deliverables:
  - benchmark budgets for common validator chains
  - cache strategy guidance (`cached`) for expensive checks
  - allocation-focused profiling for heavy failure paths
- risks:
  - regressions in deeply nested combinator usage
- exit criteria:
  - benchmark thresholds enforced in CI

## Phase 4: Ecosystem and DX

- deliverables:
  - advanced patterns for workflow/plugin/sdk consumers
  - optional schema/policy layer evaluation
  - macro debugging and authoring guidance
- risks:
  - over-expansion of API surface
- exit criteria:
  - clear stable core vs optional extension boundaries

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
