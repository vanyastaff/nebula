# Implementation Plan: nebula-validator

**Crate**: `nebula-validator` | **Path**: `crates/validator` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-validator provides an input validation framework for the Nebula workflow engine, built around composable validator combinators. Phase 1 contract/docs baseline is complete; current focus is Phase 2 governance automation with machine-readable compatibility checks and migration enforcement.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: N/A (sync validation with optional tokio in tests)
**Key Dependencies**: thiserror, serde, serde_json, regex, moka (caching), smallvec
**Testing**: `cargo test -p nebula-validator`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract and Docs Baseline | ✅ Done | Full template-aligned docs, canonical API examples, compatibility policy draft |
| Phase 2: Compatibility and Governance | 🔄 In Progress | Registry, compatibility tests, deprecation enforcement |
| Phase 3: Performance and Capacity Hardening | ⬜ Planned | Benchmarks, cache strategy, allocation profiling |
| Phase 4: Ecosystem and DX | ⬜ Planned | Advanced patterns, schema/policy layer, macro guidance |

## Phase Details

### Phase 1: Contract and Docs Baseline (Completed)

**Goal**: Establish authoritative documentation and contract baseline for the validator API.

**Deliverables**:
- Full template-aligned docs with cross-crate contracts
- Canonical API examples aligned to current implementation
- Explicit compatibility policy draft

**Exit Criteria**:
- Docs accepted as single source of truth for current API
- No stale naming mismatch in public docs

**Risks**:
- Hidden behavior assumptions in downstream crates

### Phase 2: Compatibility and Governance (In Progress)

**Goal**: Automate compatibility enforcement and governance checks for validator contracts.

**Deliverables**:
- Machine-readable error code/category registry
- Compatibility tests for error codes/field paths
- Deprecation and migration policy enforcement
- Governance checks requiring migration mapping for behavior-significant changes

**Exit Criteria**:
- Backward compatibility CI checks in place
- Migration-map checks running for release candidates

**Risks**:
- Legacy consumers relying on undocumented error details

### Phase 3: Performance and Capacity Hardening

**Goal**: Establish performance budgets and optimize hot paths.

**Deliverables**:
- Benchmark budgets for common validator/combinator chains
- Cache strategy guidance (moka `cached`) for expensive checks
- Allocation-focused profiling for heavy failure paths

**Exit Criteria**:
- Benchmark thresholds enforced in CI

**Risks**:
- Regressions in deeply nested combinator usage

### Phase 4: Ecosystem and DX

**Goal**: Expand usability for downstream consumers and plugin authors.

**Deliverables**:
- Advanced patterns for workflow/plugin/sdk consumers
- Optional schema/policy layer evaluation
- Macro debugging and authoring guidance

**Exit Criteria**:
- Clear stable core vs optional extension boundaries

**Risks**:
- Over-expansion of API surface

## Dependencies

| Depends On | Why |
|-----------|-----|
| (none) | Leaf crate with no internal dependencies |

| Depended By | Why |
|------------|-----|
| nebula-config | ConfigValidator bridge uses validator traits |
| nebula-macros | Dev-dependency for derive testing |

## Verification

- [ ] `cargo check -p nebula-validator`
- [ ] `cargo test -p nebula-validator`
- [ ] `cargo clippy -p nebula-validator -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-validator`
