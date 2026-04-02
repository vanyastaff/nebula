# Tasks: nebula-macros

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `MAC`

---

## Phase 1: Contract and Output Stability ⬜

**Goal**: Document all public derives, add contract tests, validate error messages.

- [ ] MAC-T001 [P] Document `#[derive(Action)]` — what code is generated, which attributes are stable
- [ ] MAC-T002 [P] Document `#[derive(Resource)]` — generated code, stable attributes
- [ ] MAC-T003 [P] Document `#[derive(Plugin)]` — generated code, stable attributes
- [ ] MAC-T004 [P] Document `#[derive(Credential)]` — generated code, stable attributes
- [ ] MAC-T005 [P] Document `#[derive(Parameters)]` — generated code, stable attributes
- [ ] MAC-T006 [P] Document `#[derive(Validator)]` and `#[derive(Config)]`
- [ ] MAC-T007 Add contract tests in `tests/contracts.rs` — verify each derive compiles and satisfies trait bounds
- [ ] MAC-T008 Add integration test: macro-generated action/plugin works with nebula-sdk + engine in `tests/sdk_integration.rs`
- [ ] MAC-T009 [P] Validate invalid attribute inputs produce clear compile errors — add test cases using `trybuild`

**Checkpoint**: All public derives documented; contract tests green; `trybuild` error tests pass.

---

## Phase 2: Attribute and Compatibility Hardening ⬜

**Goal**: Freeze attribute set, document compatibility matrix, handle edge cases.

- [ ] MAC-T010 Define attribute versioning policy in README — additive in minor, removal/behavior change = major
- [ ] MAC-T011 Write compatibility matrix: macro version X works with action/plugin/credential version Y
- [ ] MAC-T012 Add CI test: run macro against current nebula-action, nebula-plugin, nebula-credential
- [ ] MAC-T013 [P] Test edge cases: optional fields, generics, nested types — verify no panics in macro expansion
- [ ] MAC-T014 Document MIGRATION policy for breaking attribute changes

**Checkpoint**: Attribute policy documented; CI tests macro against live crates; edge cases covered.

---

## Phase 3: Diagnostics and DX ⬜

**Goal**: Improved compile errors, document `cargo expand` workflow, no new domain logic.

- [ ] MAC-T015 Improve error messages — suggest correct syntax when attribute is invalid
- [ ] MAC-T016 [P] Add span information to errors so rustc points to correct source location
- [ ] MAC-T017 Document `cargo expand` workflow for macro debugging in README
- [ ] MAC-T018 Audit macros for any domain logic — move to appropriate crates if found

**Checkpoint**: Authors get actionable errors with correct spans; `cargo expand` documented.

---

## Phase 4: Ecosystem and Versioning ⬜

**Goal**: Version alignment with platform, stable re-export via nebula-sdk.

- [ ] MAC-T019 Align macro crate version with platform releases — document in MIGRATION for breaking changes
- [ ] MAC-T020 Verify re-export through `nebula-sdk` prelude works correctly
- [ ] MAC-T021 [P] Ensure no duplicate macros in workspace (single source of truth)
- [ ] MAC-T022 Add version compatibility docs in README — "use nebula-macros X with nebula-action Y"

**Checkpoint**: SDK prelude gives compatible macro output by default; version docs published.

---

## Dependencies & Execution Order

- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- [P] tasks within each phase can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-macros`
- [ ] `cargo test -p nebula-macros`
- [ ] `cargo clippy -p nebula-macros -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-macros`
