# Implementation Plan: nebula-core

**Crate**: `nebula-core` | **Path**: `crates/core` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

`nebula-core` provides the foundational identifiers, scope system, shared traits, and error types for the entire Nebula workspace. Phases 1-3 (API cleanup, compatibility contracts, scope hardening) are complete. Current focus is on constants governance and MSRV strategy.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: None (sync crate; Tokio in dev-dependencies only)
**Key Dependencies**: `serde`, `serde_json`, `chrono`, `thiserror`, `domain-key`, `postcard`
**Testing**: `cargo test -p nebula-core`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: API & Docs Cleanup | ✅ Done | Public docs aligned with source behavior |
| Phase 2: Compatibility Contracts | ✅ Done | Schema contract tests enforced in CI |
| Phase 3: Scope Semantics Hardening | ✅ Done | ScopeResolver trait, strict containment APIs |
| Phase 4: Constants Governance | ⬜ Planned | Split constants into tiers (P-003) |
| Phase 5: Rust Baseline Strategy | ⬜ Planned | MSRV bump path beyond 1.93 |

## Phase Details

### Phase 1: API & Docs Cleanup

**Goal**: Align all public documentation with exact source behavior and examples.

**Deliverables**:
- Align public docs (`API.md`, `CONSTITUTION.md`, crate `README.md`) with exact source behavior
- Remove outdated API narratives from archived docs
- Audit naming consistency (`id.rs` vs `ids`, trait method names, scope terminology)
- Add missing module-level examples for `keys`, `scope`, and `CoreError` usage

**Exit Criteria**:
- All public docs conceptually match current APIs (`lib.rs` re-exports and module maps)
- No stale references to non-existing methods or types

### Phase 2: Compatibility Contracts

**Goal**: Establish explicit compatibility policies and enforce them via CI.

**Deliverables**:
- Explicit compatibility policy in `COMPATIBILITY.md` for `InterfaceVersion`, serialized enums, `ScopeLevel`, ID types
- `CoreError::error_code()` stability guarantees
- Schema contract tests in `crates/core/tests/schema_contracts.rs`

**Exit Criteria**:
- Breaking-change rules documented and test-enforced for IDs, enums, and core types used across crate/API/storage boundaries

### Phase 3: Scope Semantics Hardening

**Goal**: Make containment rules explicit, test-covered, and unambiguous.

**Deliverables**:
- `ScopeLevel::is_contained_in` retained as simplified level-only check (backward compatible)
- `ScopeResolver` trait and `is_contained_in_strict(scope, other, resolver)` added for ID-verified containment
- Canonical scope hierarchy and transitions documented in `ARCHITECTURE.md`

**Exit Criteria**:
- Containment rules are explicit, test-covered, and unambiguous (including strict/ID-aware APIs)

### Phase 4: Constants Governance

**Goal**: Split broad constants into tiers so `constants.rs` contains only stable foundation defaults.

**Deliverables**:
- Split constants into tiers: truly global/cross-cutting defaults (keep in core) vs domain-owned defaults (move to owning crate)
- Mark deprecated constants and provide migration notes and aliases

**Exit Criteria**:
- `constants.rs` contains only stable foundation defaults
- Domain constants live in owning crates with documented migration

**Risks**:
- Downstream crates may depend on constants currently in core; migration requires coordination

**Dependencies**: None (core is the foundation crate)

### Phase 5: Rust Baseline Strategy

**Goal**: Prepare MSRV bump path beyond Rust 1.93 with explicit policy.

**Deliverables**:
- CI matrix update for new MSRV
- Clippy/rustdoc policy checks
- Documentation refresh for language/library changes

**Exit Criteria**:
- Workspace baseline updated with green CI, updated docs, and documented MSRV strategy for future upgrades

**Risks**:
- New Rust editions may introduce breaking changes in existing code

**Dependencies**: None (workspace-wide concern, but owned by core)

## Inter-Crate Dependencies

- **Depends on**: (none -- foundation crate)
- **Depended by**: nebula-workflow, nebula-execution, nebula-memory, nebula-expression, nebula-parameter (indirect via validator), and most other crates

## Verification

- [ ] `cargo check -p nebula-core`
- [ ] `cargo test -p nebula-core`
- [ ] `cargo clippy -p nebula-core -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-core`
