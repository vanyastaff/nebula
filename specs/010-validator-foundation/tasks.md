# Tasks: Validator Foundation Restructuring

**Input**: Design documents from `/specs/010-validator-foundation/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/public-api.rs, quickstart.md
**Branch**: `010-validator-foundation`

**Tests**: TDD approach for new validators (Phase 4). Existing 500+ tests serve as regression suite.

**Organization**: Tasks follow the 6-phase implementation plan (A-F). User Stories 3 and 4 (feature flags, clean structure) are satisfied by the Foundational phase since they are prerequisites for all other stories.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2)
- Exact file paths relative to `crates/validator/`

---

## Phase 1: Setup

**Purpose**: Establish baseline and verify the crate is in a known-good state before restructuring.

- [X] T001 Verify baseline: run `cargo test -p nebula-validator` and `cargo clippy -p nebula-validator -- -D warnings` to confirm all tests pass and no warnings exist before any changes

**Checkpoint**: Crate compiles, all tests pass. Ready to begin restructuring.

---

## Phase 2: Foundational (Dead Code, Rename, Flatten, Feature Flags)

**Purpose**: All structural changes that MUST complete before user story implementation. Satisfies **US3** (feature flags) and **US4** (flat modern crate) acceptance criteria.

**CRITICAL**: No user story work (prelude, json, new validators) can begin until this phase is complete.

### Sub-phase 2a: Dead Code Removal (Plan Phase A, Steps 1-2)

> Removes ~650 LOC: AsyncValidate trait, Refined<T,V>, Parameter<T,S> type-state, Map combinator.
> Satisfies FR-006, FR-007, FR-008, FR-009.

- [X] T002 [P] Delete dead code files: `src/core/refined.rs`, `src/core/state.rs`, `src/combinators/map.rs`, `tests/refined_test.rs`
- [X] T003 [P] Remove dead code references: remove `pub mod refined`, `pub mod state`, re-exports of Refined/Parameter/AsyncValidate from `src/core/mod.rs`; remove `AsyncValidate` trait definition and `.map()` method from ValidateExt in `src/core/traits.rs`; remove `pub mod map` and all Map/map re-exports from `src/combinators/mod.rs`
- [X] T004 Verify compilation after dead code removal: `cargo check -p nebula-validator`

### Sub-phase 2b: Core to Foundation Rename (Plan Phase A, Step 3)

> Renames `core/` to `foundation/` with NO deprecated alias. Clean break.
> Satisfies FR-001, SC-007.

- [X] T005 Rename directory `src/core/` to `src/foundation/` using `git mv crates/validator/src/core crates/validator/src/foundation`
- [X] T006 Update `src/lib.rs`: change `pub mod core` to `pub mod foundation`. Update all source files under `src/`: replace `crate::core::` with `crate::foundation::` globally
- [X] T007 Update all test files in `tests/`, benchmark files in `benches/`, and example files in `examples/`: replace `nebula_validator::core::` with `nebula_validator::foundation::`
- [X] T008 Verify all tests pass after rename: `cargo test -p nebula-validator`

### Sub-phase 2c: Flatten Validators (Plan Phase B)

> Moves 27 validator files from 5 subcategory folders to flat `validators/` directory.
> Satisfies FR-002, SC-009.

- [X] T009 Move all 14 files from `src/validators/string/` to `src/validators/` using `git mv` (rename `json.rs` to `json_string.rs` to avoid conflict with top-level json module). Files: length.rs, pattern.rs, content.rs, uuid.rs, datetime.rs, json.rs->json_string.rs, password.rs, phone.rs, credit_card.rs, iban.rs, semver.rs, slug.rs, hex.rs, base64.rs
- [X] T010 [P] Move all 5 files from `src/validators/numeric/` to `src/validators/` using `git mv`. Files: range.rs, properties.rs, divisibility.rs, float.rs, percentage.rs
- [X] T011 [P] Move all 3 files from `src/validators/collection/` to `src/validators/` using `git mv`. Files: size.rs, elements.rs, structure.rs
- [X] T012 [P] Move all 3 files from `src/validators/network/` to `src/validators/` using `git mv`. Files: ip_address.rs, port.rs, mac_address.rs
- [X] T013 [P] Move all 2 files from `src/validators/logical/` to `src/validators/` using `git mv`. Files: boolean.rs, nullable.rs
- [X] T014 Delete empty subcategory directories and their `mod.rs` files: `src/validators/string/`, `src/validators/numeric/`, `src/validators/collection/`, `src/validators/network/`, `src/validators/logical/`
- [X] T015 Rewrite `src/validators/mod.rs` with flat module declarations (one `pub mod` per file) and consolidated re-exports matching contracts/public-api.rs
- [X] T016 Update all internal imports across source files: change subcategory paths (e.g., `crate::validators::string::length` to `crate::validators::length`, `use super::super::` patterns to `use super::` or `use crate::`)
- [X] T017 Verify all tests pass after flattening: `cargo test -p nebula-validator`

### Sub-phase 2d: Feature Flags (Plan Phase C)

> Gates optional components behind feature flags. moka becomes optional.
> Satisfies FR-010, FR-011, FR-012, SC-002, SC-004.

- [X] T018 Update `crates/validator/Cargo.toml`: add `[features]` section with `default = ["serde"]`, `serde = []`, `caching = ["dep:moka"]`, `optimizer = []`, `full = ["serde", "caching", "optimizer"]`. Change moka dependency to `optional = true`
- [X] T019 [P] Gate caching behind `#[cfg(feature = "caching")]`: wrap entire `src/combinators/cached.rs` module, conditional `pub mod cached` and re-exports in `src/combinators/mod.rs`, gate `.cached()` and `.cached_with_capacity()` methods in `src/foundation/traits.rs` (ValidateExt)
- [X] T020 [P] Gate optimizer behind `#[cfg(feature = "optimizer")]`: wrap entire `src/combinators/optimizer.rs` module, conditional `pub mod optimizer` and re-exports in `src/combinators/mod.rs`, gate `ValidatorStatistics` and `RegisteredValidatorMetadata` in `src/foundation/metadata.rs`
- [X] T021 Verify serde gating: ensure `AsValidatable<_> for Value` impls in `src/foundation/validatable.rs` and `JsonField` in `src/combinators/json_field.rs` are behind `#[cfg(feature = "serde")]`. Add gates if missing
- [X] T022 Gate `tests/optimizer_test.rs` behind `#[cfg(feature = "optimizer")]` attribute on the entire module
- [X] T023 Verify all 4 feature combinations compile: `cargo check -p nebula-validator --no-default-features`, `cargo check -p nebula-validator` (default=serde), `cargo check -p nebula-validator --features caching`, `cargo check -p nebula-validator --all-features`

**Checkpoint**: Crate has clean structure (no dead code, foundation/ module, flat validators, feature flags). US3 and US4 acceptance criteria verifiable. All existing tests pass.

---

## Phase 3: User Story 1 - Parameter Developer Uses Validator via Prelude (Priority: P1)

**Goal**: Provide a single-import prelude and turbofish-free JSON collection validators so consumers can use the crate with `use nebula_validator::prelude::*`.

**Independent Test**: Import `nebula_validator::prelude::*`, call `min_length(5).validate_any(&json!("hello"))` and `json_min_size(2).validate_any(&json!([1,2,3]))` — both should work without turbofish.

**Satisfies**: FR-003, FR-004, FR-005, SC-001, SC-008.

### Implementation for User Story 1

- [X] T024 [P] [US1] Create `crates/validator/src/json.rs` behind `#[cfg(feature = "serde")]`: define type aliases `JsonMinSize`, `JsonMaxSize`, `JsonExactSize`, `JsonSizeRange` and factory functions `json_min_size()`, `json_max_size()`, `json_exact_size()`, `json_size_range()` per contracts/public-api.rs
- [X] T025 [P] [US1] Create `crates/validator/src/prelude.rs`: re-export all foundation traits (Validate, ValidateExt, AsValidatable, ValidationError, ValidationErrors, ErrorSeverity, ValidatorMetadata, ValidationComplexity), all validator factory functions, combinator functions, and `#[cfg(feature = "serde")] pub use crate::json::*` per contracts/public-api.rs
- [X] T026 [US1] Update `crates/validator/src/lib.rs`: add `pub mod prelude` and `#[cfg(feature = "serde")] pub mod json`
- [X] T027 [US1] Write integration test in `crates/validator/tests/prelude_test.rs`: verify prelude import, `validate_any()` on `serde_json::Value` types (String, Number, Array), `json_min_size()` without turbofish, type_mismatch error on `Value::Null`
- [X] T028 [US1] Verify: `cargo test -p nebula-validator --all-features` and `cargo check -p nebula-validator --no-default-features` (prelude works without serde, json module excluded)

**Checkpoint**: Consumer can `use nebula_validator::prelude::*` and validate JSON values with zero turbofish. US1 fully functional.

---

## Phase 4: User Story 2 - Config Developer Validates Formats (Priority: P1)

**Goal**: Implement three new validators (Hostname, TimeOnly, DateTime::date_only) so config developers have production-quality format validation.

**Independent Test**: Call `hostname().validate("api.example.com")`, `time_only().validate("14:30:00")`, and `DateTime::date_only().validate("2026-02-16")` — all should pass. Invalid inputs should fail with descriptive errors.

**Satisfies**: FR-016, FR-017, FR-018, SC-005.

### Implementation for User Story 2 (TDD)

> Write tests FIRST per each validator, verify they fail, then implement.

- [X] T029 [P] [US2] Implement Hostname validator (RFC 1123) in `crates/validator/src/validators/hostname.rs`: write tests first (valid hostnames, leading/trailing hyphen, >253 chars, >63 char label, empty, single dot, double dot, trailing dot FQDN), then implement per research.md R7. Add `hostname()` factory function
- [X] T030 [P] [US2] Implement TimeOnly validator in `crates/validator/src/validators/time.rs`: write tests first (valid HH:MM:SS, with milliseconds, with timezone, 25:00:00 rejected, 00:60:00 rejected, empty string rejected), then implement per research.md R8. Add `time_only()` factory and `.require_timezone()` builder
- [X] T031 [US2] Add `DateTime::date_only()` builder to `crates/validator/src/validators/datetime.rs`: write tests first ("2026-02-16" passes, "2026-02-16T10:00:00" rejected, "2026-13-01" rejected), then implement per research.md R9
- [X] T032 [US2] Register new validators: add `pub mod hostname` and `pub mod time` to `crates/validator/src/validators/mod.rs`, add re-exports of `hostname()`, `time_only()`, `Hostname`, `TimeOnly` in `validators/mod.rs` and `crates/validator/src/prelude.rs`
- [X] T033 [US2] Verify: `cargo test -p nebula-validator --all-features` — all new and existing tests pass

**Checkpoint**: Three new validators implemented with full test coverage. US2 fully functional.

---

## Phase 5: User Story 5 - CI Quality Gates (Priority: P3)

**Goal**: Verify the crate passes all CI checks across all feature combinations.

**Independent Test**: Run the full quality gate command suite and confirm zero errors, zero warnings.

**Satisfies**: FR-019, FR-020, FR-021, SC-002, SC-003, SC-006, SC-010.

### Verification for User Story 5

- [X] T034 [P] [US5] Run `cargo fmt --all -- --check` and fix any formatting issues in `crates/validator/`
- [X] T035 [P] [US5] Run clippy across all feature combinations: `cargo clippy -p nebula-validator -- -D warnings` and `cargo clippy -p nebula-validator --all-features -- -D warnings`
- [X] T036 [US5] Run `cargo check -p nebula-validator --no-default-features` to verify serde-dependent code is fully gated
- [X] T037 [US5] Run `cargo test -p nebula-validator` (default features) and `cargo test -p nebula-validator --all-features`
- [X] T038 [US5] Run `cargo doc --no-deps -p nebula-validator` to verify documentation builds
- [X] T039 [US5] Run `cargo test --workspace` to verify no impact on other crates (`nebula-parameter`, `nebula-config`, etc.)
- [X] T040 [US5] Verify all success criteria SC-001 through SC-010 from spec.md: confirm foundation/ path works, core/ doesn't compile, flat validators, no dead code in API, json helpers match turbofish equivalents

**Checkpoint**: All CI quality gates pass. Crate is production-ready.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final cleanup affecting multiple user stories.

- [X] T041 [P] Update example files in `crates/validator/examples/` to use `foundation::` paths and `prelude::*` imports
- [X] T042 [P] Update benchmark files in `crates/validator/benches/` to use `foundation::` paths
- [X] T043 Final comprehensive verification: `cargo test --workspace` and `cargo clippy --workspace -- -D warnings`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 — **BLOCKS all user stories**
  - Sub-phase 2a (dead code) → 2b (rename) → 2c (flatten) → 2d (feature flags) — sequential within phase
- **US1 Prelude (Phase 3)**: Depends on Phase 2 completion (needs flat validators + feature flags)
- **US2 New Validators (Phase 4)**: Depends on Phase 2 completion (needs flat validators directory)
  - US1 and US2 **CAN run in parallel** after Phase 2 (different files)
- **US5 Quality Gates (Phase 5)**: Depends on Phase 3 + Phase 4 completion
- **Polish (Phase 6)**: Depends on Phase 5 completion

### User Story Dependencies

- **US3** (Feature Flags, P2): Satisfied by Phase 2, sub-phase 2d — no separate story phase needed
- **US4** (Flat Modern Crate, P2): Satisfied by Phase 2, sub-phases 2a-2c — no separate story phase needed
- **US1** (Prelude, P1): Phase 3 — can start after Phase 2
- **US2** (New Validators, P1): Phase 4 — can start after Phase 2, independent of US1
- **US5** (CI Gates, P3): Phase 5 — after all implementation complete

### Within Each Phase

- Tasks marked [P] within the same sub-phase can run in parallel
- Verify/checkpoint tasks must wait for all preceding tasks in the sub-phase
- Sequential sub-phases (2a → 2b → 2c → 2d) must complete in order

### Parallel Opportunities

Within Phase 2:
- T002 + T003 (delete files + remove references — different files)
- T010 + T011 + T012 + T013 (move numeric + collection + network + logical — different directories)
- T019 + T020 (gate caching + gate optimizer — different files)

Between Phases 3 and 4 (after Phase 2 completes):
- T024 + T025 (json.rs + prelude.rs) can run in parallel with T029 + T030 (hostname + time validators)

Within Phase 5:
- T034 + T035 (fmt + clippy — independent checks)

Within Phase 6:
- T041 + T042 (examples + benchmarks — different directories)

---

## Parallel Example: After Phase 2 Completes

```text
# US1 and US2 can run concurrently:

Agent A (US1 - Prelude):          Agent B (US2 - Validators):
  T024: Create json.rs              T029: Implement Hostname
  T025: Create prelude.rs           T030: Implement TimeOnly
  T026: Update lib.rs               T031: Add date_only()
  T027: Write prelude test           T032: Register in mod.rs
  T028: Verify                       T033: Verify
```

---

## Implementation Strategy

### MVP First (Phase 1 + 2 + 3)

1. Complete Phase 1: Setup (baseline)
2. Complete Phase 2: Foundational (dead code, rename, flatten, feature flags)
3. Complete Phase 3: User Story 1 (prelude + json)
4. **STOP and VALIDATE**: The crate is consumable via `prelude::*` with clean structure
5. This alone delivers value: parameter/config teams can start integration

### Incremental Delivery

1. Phase 1 + 2 → Clean, modern crate structure (US3 + US4 done)
2. Add Phase 3 → Prelude + JSON convenience (US1 done, **MVP!**)
3. Add Phase 4 → New format validators (US2 done)
4. Add Phase 5 → CI verification (US5 done)
5. Add Phase 6 → Final polish
6. Each phase adds value without breaking previous phases

### Research Findings (No Action Needed)

Per research.md:
- **FR-014** (simplify Cow): Research R1 concluded `Cow<'static, str>` is already optimal — `WithCode`/`WithMessage` combinators need runtime strings. **No changes needed.**
- **FR-015** (simplify GAT): Research R2 concluded GAT in `AsValidatable` is already the idiomatic Rust 2024 pattern. **No changes needed.**

---

## Notes

- [P] tasks = different files, no dependencies on each other
- [Story] label maps task to specific user story for traceability
- US3 and US4 are implicitly satisfied by Phase 2 (Foundational) — all their acceptance criteria are verifiable after Phase 2 checkpoint
- Use `git mv` for all file moves to preserve history
- Commit after each sub-phase checkpoint for safe rollback points
- Estimated ~650 LOC removed, 27 files moved, 3 new files created, ~5 files significantly rewritten
- All tasks reference exact file paths relative to `crates/validator/`
