# Roadmap

## Status Snapshot

- completed:
  - Phase 1 baseline docs/contracts.
  - Phase 2 governance automation (error registry, compatibility fixtures, migration policy).
  - Phase 3 performance hardening (benchmark budgets, bench profiles, threshold policy, cache/error benchmarks).
  - Phase 4 ecosystem and DX (patterns, schema evaluation, macro guide, stability boundaries).
  - Phase 5 stabilization and integration (ValidationMode, FieldPath, SelfValidating, prelude expansion).
- next focus:
  - Phase 6 (planned): attribute macros, schema bridge, async validation.

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

## Phase 2: Compatibility and Governance (Complete)

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

## Phase 3: Performance and Capacity Hardening (Complete)

- deliverables:
  - benchmark budgets for common validator/combinator chains
  - cache strategy guidance (`cached`) for expensive checks
  - allocation-focused profiling for heavy failure paths
- risks:
  - regressions in deeply nested combinator usage
- exit criteria:
  - benchmark thresholds enforced in CI

## Phase 4: Ecosystem and DX (Complete)

- deliverables:
  - advanced patterns for workflow/plugin/sdk consumers (`PATTERNS.md`)
  - optional schema/policy layer evaluation (`SCHEMA_EVALUATION.md` — defer)
  - macro debugging and authoring guidance (`MACROS.md`)
  - stable core vs extension boundary (`BOUNDARIES.md`)
- risks:
  - over-expansion of API surface
- exit criteria:
  - clear stable core vs optional extension boundaries ✅

## Phase 5: Stabilization and Integration (Complete)

- deliverables:
  - `ValidationMode` enum (FailFast / CollectAll) for AllOf, MultiField, Each, CollectionNested
  - `SelfValidating` trait (renamed from nested `Validatable`) with `check()` method
  - `FieldPath` type — validated RFC 6901 JSON Pointer with segment access and composition
  - `with_field_path(FieldPath)` builder on `ValidationError`
  - expanded prelude with all combinator types and factory functions
  - `collection_nested_failed` error code in registry
- risks:
  - `SelfValidating` rename is breaking for nested `Validatable` consumers
- exit criteria:
  - all combinators support configurable ValidationMode ✅
  - FieldPath provides typed path operations ✅
  - 479+ tests passing, clippy clean, 0 doc warnings ✅

## Phase 6: Ecosystem Expansion (Planned)

- deliverables:
  - `#[validate(...)]` attribute macro for derive-style validation (P005)
  - schema bridge: JSON Schema ↔ validator conversion (P004)
  - async validation support
  - cross-crate integration tests (nebula-config, nebula-workflow)

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
