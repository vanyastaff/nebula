# Tasks: nebula-core

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix COR (3-letter crate code)

---

## Phase 1: API & Docs Cleanup

**Goal**: Align public docs with source behavior

- [x] COR-T001 [P] Align public docs (`API.md`, `CONSTITUTION.md`, `README.md`) with exact source behavior and examples
- [x] COR-T002 [P] Remove outdated API narratives from archived docs
- [x] COR-T003 [P] Audit naming consistency (`id.rs` vs `ids`, trait method names, scope terminology)
- [x] COR-T004 [P] Add missing module-level examples for `keys`, `scope`, and `CoreError` usage

**Checkpoint**: All public docs match current APIs; no stale references to non-existing methods or types.

---

## Phase 2: Compatibility Contracts

**Goal**: Establish explicit compatibility policies enforced via CI

- [x] COR-T005 [P] Write `COMPATIBILITY.md` covering `InterfaceVersion`, serialized enums, `ScopeLevel`, ID types
- [x] COR-T006 [P] Document `CoreError::error_code()` stability guarantees
- [x] COR-T007 Create schema contract tests in `crates/core/tests/schema_contracts.rs` (depends on T005, T006)
- [x] COR-T008 Verify CI enforces schema contract tests on every PR

**Checkpoint**: Breaking-change rules documented and test-enforced for IDs, enums, and core types.

---

## Phase 3: Scope Semantics Hardening

**Goal**: Make containment rules explicit and unambiguous

- [x] COR-T009 [P] Retain `ScopeLevel::is_contained_in` as simplified level-only check
- [x] COR-T010 [P] Implement `ScopeResolver` trait for ID-verified containment in `src/scope.rs`
- [x] COR-T011 Implement `is_contained_in_strict(scope, other, resolver)` function (depends on T010)
- [x] COR-T012 Document canonical scope hierarchy and transitions in `ARCHITECTURE.md`
- [x] COR-T013 Write tests covering strict/ID-aware containment APIs (depends on T011)

**Checkpoint**: Containment rules are explicit, test-covered, and unambiguous.

---

## Phase 4: Constants Governance

**Goal**: Split `constants.rs` into tiers; move domain constants to owning crates

- [ ] COR-T014 [P] Audit `src/constants.rs` and classify each constant as global or domain-owned
- [ ] COR-T015 [P] Identify all downstream crate usages of each constant via workspace-wide grep
- [ ] COR-T016 Define migration plan for domain constants moving to owning crates (depends on T014, T015)
- [ ] COR-T017 Mark deprecated constants with `#[deprecated]` and migration notes in `src/constants.rs` (depends on T016)
- [ ] COR-T018 Create re-export aliases for backward compatibility during transition (depends on T017)
- [ ] COR-T019 Move domain-owned constants to their respective owning crates (depends on T016)
- [ ] COR-T020 Update `MIGRATION.md` to document new constant locations (depends on T019)
- [ ] COR-T021 Verify no downstream breakage with `cargo check --workspace` after constant migration (depends on T019)

**Checkpoint**: `constants.rs` contains only stable foundation defaults; domain constants live in owning crates.

---

## Phase 5: Rust Baseline Strategy

**Goal**: Prepare MSRV bump path beyond Rust 1.93

- [ ] COR-T022 [P] Document current MSRV policy and bump criteria in `CONSTITUTION.md`
- [ ] COR-T023 [P] Update CI matrix to test against current and next Rust versions
- [ ] COR-T024 Review clippy/rustdoc policy checks for new Rust version compatibility (depends on T023)
- [ ] COR-T025 Refresh documentation for any language/library changes introduced by new Rust versions (depends on T024)
- [ ] COR-T026 Verify green CI across all workspace crates after MSRV update (depends on T025)

**Checkpoint**: Workspace baseline updated with green CI, updated docs, and documented MSRV strategy.

---

## Dependencies & Execution Order

- Phases 1-3 are complete
- Phase 4 and Phase 5 can run in parallel (no cross-dependency)
- Within each phase, [P] tasks can run in parallel
- Non-[P] tasks have explicit dependency chains noted above
