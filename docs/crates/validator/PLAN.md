# Implementation Plan: nebula-validator

**Crate**: `nebula-validator` | **Path**: `crates/validator` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

nebula-validator provides an input validation framework for the Nebula workflow engine, built around composable validator combinators. Phases 1–4 are complete. Phase 5 (Stabilization & Integration) is in progress, adding `ValidationMode`, typed `FieldPath`, and stabilizing experimental APIs.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: N/A (sync validation with optional tokio in tests)
**Key Dependencies**: thiserror, serde, serde_json, regex, moka (caching), smallvec
**Testing**: `cargo test -p nebula-validator`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract and Docs Baseline | ✅ Done | Full template-aligned docs, canonical API examples, compatibility policy draft |
| Phase 2: Compatibility and Governance | ✅ Done | Registry, compatibility tests, deprecation enforcement |
| Phase 3: Performance and Capacity Hardening | ✅ Done | Benchmarks, cache strategy, allocation profiling |
| Phase 4: Ecosystem and DX | ✅ Done | Advanced patterns, schema/policy layer, macro guidance |
| Phase 5: Stabilization and Integration | ✅ Done | ValidationMode, FieldPath, SelfValidating, prelude expansion |

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

### Phase 5: Stabilization and Integration (Completed)

**Goal**: Consolidate experimental APIs, add missing cross-cutting features, and expand the prelude.

**Deliverables**:
- `ValidationMode` enum (FailFast / CollectAll) integrated into AllOf, MultiField, Each, CollectionNested
- Renamed `Validatable` → `SelfValidating` (trait for self-validating types, method `check()`)
- `FieldPath` type — validated, zero-overhead RFC 6901 JSON Pointer wrapper with segment access, composition, and parsing
- `with_field_path(FieldPath)` builder on `ValidationError`
- Expanded prelude with AllOf, AnyOf, Each, Field, MultiField, NestedValidate, OptionalNested, CollectionNested, SelfValidating, FieldPath, ValidationMode
- `collection_nested_failed` error code registered

**Exit Criteria**:
- All combinators support configurable ValidationMode
- FieldPath provides typed path construction, segment iteration, and composition
- Prelude is comprehensive enough for most use cases
- 479+ tests passing, clippy clean, 0 doc warnings

**Risks**:
- SelfValidating rename is a breaking change for consumers using the old `Validatable` (nested) trait

## Future Phases

### Phase 6: Ecosystem Expansion (Planned)

- P005: `#[validate(...)]` attribute macro for derive-style validation
- P004: Schema bridge (JSON Schema ↔ validator conversion)
- Async validation support
- Cross-crate integration tests with nebula-config, nebula-workflow

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
