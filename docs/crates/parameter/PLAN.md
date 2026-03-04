# Implementation Plan: nebula-parameter

**Crate**: `nebula-parameter` | **Path**: `crates/parameter` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The parameter crate defines the schema for workflow node parameters — typed fields, validation rules, display conditions, and schema lint. It targets stronger contracts between schema, UI, and runtime execution. Current focus is Phase 1: aligning docs with API, publishing stable naming conventions, and schema lint.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: N/A (sync)
**Key Dependencies**: `nebula-core`, `serde`, `serde_json`
**Testing**: `cargo test -p nebula-parameter`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and Safety Baseline | ⬜ Planned | Align docs, stable naming, schema lint |
| Phase 2: Runtime Hardening | ⬜ Planned | Benchmarks, deterministic error ordering |
| Phase 3: Scale and Performance | ⬜ Planned | Typed extraction helpers, typed value layer |
| Phase 4: Ecosystem and DX | ⬜ Planned | Display rule analysis, key linting, versioning |

## Phase Details

### Phase 1: Contract and Safety Baseline

**Goal**: Align docs/examples with actual API; stable naming for keys/paths; document required vs nullable; schema lint.

**Deliverables**:
- Docs and examples aligned with actual API
- Stable naming conventions for keys/paths published
- Required vs nullable behavior documented per kind
- Schema lint pass (P-004)

**Exit Criteria**:
- All consumers pass lint; error code stability documented

**Risks**:
- Lint may surface breaking schema issues in existing definitions

### Phase 2: Runtime Hardening

**Goal**: Benchmark deep nested validation; stress tests; deterministic error ordering.

**Deliverables**:
- Criterion benchmarks for deep nested object/list validation
- Optimized recursive path building and error allocation
- Stress tests for large collections and high error counts
- Deterministic error ordering (P-002)

**Exit Criteria**:
- Benchmarks in CI; no allocation regression in common cases

### Phase 3: Scale and Performance

**Goal**: Typed extraction helpers; clearer conversion contracts; optional typed value layer.

**Deliverables**:
- Typed extraction helpers for common types
- Clearer conversion contracts for numbers/integers/decimals
- Reduced ambiguity in "any"-typed flows
- Optional typed value layer (P-001)

**Exit Criteria**:
- Typed API available; migration path documented

### Phase 4: Ecosystem and DX

**Goal**: Display rule dependency analysis; key linting; `ParameterKey` newtype; `ValidationRule` versioning.

**Deliverables**:
- Dependency graph extraction from display rules
- Cycle/contradictory visibility detection at schema build time
- Diagnostics for unreachable parameters
- `ValidationRule` versioning (P-005)
- `ParameterKey` newtype (P-003)

**Exit Criteria**:
- Display rule lint; version metadata in persisted schemas

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `serde_json`
- **Depended by**: `nebula-action` (parameter schema for nodes), `nebula-expression`, `nebula-sdk`, `nebula-api`

## Verification

- [ ] `cargo check -p nebula-parameter`
- [ ] `cargo test -p nebula-parameter`
- [ ] `cargo clippy -p nebula-parameter -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-parameter`
