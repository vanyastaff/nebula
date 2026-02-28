# Tasks: Validator Contract Hardening

**Input**: Design documents from `/specs/001-validator-crate-spec/`  
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: Included. Spec explicitly requires independent test criteria and contract/performance/security verification.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on incomplete task)
- **[Story]**: User story label (`[US1]`, `[US2]`, `[US3]`, `[US4]`)
- Every task includes explicit file path(s)

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish contract-testing skeleton and documentation entry points.

- [X] T001 Create validator contract test directory structure in `crates/validator/tests/contract/` and fixture folder `crates/validator/tests/fixtures/compat/`
- [X] T002 Add contract test module wiring in `crates/validator/tests/integration_test.rs` and `crates/validator/tests/mod.rs` (create if missing)
- [X] T003 [P] Add feature-level task tracker references in `specs/001-validator-crate-spec/quickstart.md`
- [X] T004 [P] Add validator contract hardening section in `docs/crates/validator/README.md` linking API, migration, and compatibility artifacts

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build shared foundations required by all user stories.

**⚠️ CRITICAL**: No story work begins before this phase is complete.

- [X] T005 Define canonical error code catalog baseline in `docs/crates/validator/API.md` and `docs/crates/validator/DECISIONS.md`
- [X] T006 Define field-path format contract and examples in `docs/crates/validator/API.md` and `docs/crates/validator/INTERACTIONS.md`
- [X] T007 [P] Add compatibility fixture schema/readme in `crates/validator/tests/fixtures/compat/README.md`
- [X] T008 [P] Add shared contract assertion helpers in `crates/validator/tests/contract/helpers.rs`
- [X] T009 Add migration contract template for breaking semantic changes in `docs/crates/validator/MIGRATION.md`
- [X] T010 Add CI-oriented validator quality gate commands to `docs/crates/validator/TEST_STRATEGY.md`

**Checkpoint**: Foundation ready for independent user story implementation.

---

## Phase 3: User Story 1 - Stable Validation Contract for Consumers (Priority: P1) 🎯 MVP

**Goal**: Guarantee stable validator behavior, error codes, and field paths across minor releases.

**Independent Test**: Run consumer-facing compatibility fixtures and verify pass/fail outcomes plus error-code/field-path equality.

### Tests for User Story 1

- [X] T011 [P] [US1] Add compatibility fixture tests for stable outcomes in `crates/validator/tests/contract/compatibility_fixtures_test.rs`
- [X] T012 [P] [US1] Add cross-entry-point equivalence tests (`validate` vs `validate_any`) in `crates/validator/tests/contract/typed_dynamic_equivalence_test.rs`
- [X] T013 [US1] Add fixture data files for valid/invalid cases in `crates/validator/tests/fixtures/compat/minor_contract_v1.json`

### Implementation for User Story 1

- [X] T014 [P] [US1] Normalize stable error-code mappings in `crates/validator/src/foundation/error.rs`
- [X] T015 [P] [US1] Ensure field-path propagation consistency in `crates/validator/src/combinators/field.rs` and `crates/validator/src/combinators/json_field.rs`
- [X] T016 [US1] Add contract-level public API examples aligned with fixtures in `docs/crates/validator/API.md`
- [X] T017 [US1] Document consumer mapping expectations for `api/workflow/plugin/runtime` in `docs/crates/validator/INTERACTIONS.md`
- [X] T018 [US1] Add minor-version compatibility policy wording in `docs/crates/validator/README.md` and `docs/crates/validator/MIGRATION.md`

**Checkpoint**: US1 is independently testable as MVP contract baseline.

---

## Phase 4: User Story 2 - Predictable Composition Semantics Under Load (Priority: P1)

**Goal**: Preserve deterministic combinator semantics and maintain performance budgets for hot paths.

**Independent Test**: Execute combinator semantic tests and benchmark scenarios for normal/adversarial inputs with budget assertions.

### Tests for User Story 2

- [X] T019 [P] [US2] Add deterministic combinator semantics tests (`and/or/not/when/unless`) in `crates/validator/tests/contract/combinator_semantics_contract_test.rs`
- [X] T020 [P] [US2] Add adversarial nested/regex scenarios in `crates/validator/tests/contract/adversarial_inputs_test.rs`
- [X] T021 [P] [US2] Add performance baseline bench assertions for hot chains in `crates/validator/benches/combinators.rs`

### Implementation for User Story 2

- [X] T022 [P] [US2] Align short-circuit behavior docs and invariants in `crates/validator/src/combinators/and.rs` and `crates/validator/src/combinators/or.rs`
- [X] T023 [P] [US2] Ensure `cached` optimization does not alter correctness in `crates/validator/src/combinators/cached.rs`
- [X] T024 [US2] Add bounded error-tree handling notes and examples in `docs/crates/validator/RELIABILITY.md` and `docs/crates/validator/SECURITY.md`
- [X] T025 [US2] Update benchmark budget policy and regression gate text in `docs/crates/validator/TEST_STRATEGY.md`

**Checkpoint**: US2 combinator determinism and load behavior are independently verified.

---

## Phase 5: User Story 3 - Actionable and Safe Validation Diagnostics (Priority: P2)

**Goal**: Provide structured and actionable diagnostics without sensitive data leakage.

**Independent Test**: Validate structured error payload shape, deterministic fields, and non-leakage constraints across representative failures.

### Tests for User Story 3

- [X] T026 [P] [US3] Add error-envelope schema validation tests in `crates/validator/tests/contract/error_envelope_schema_test.rs`
- [X] T027 [P] [US3] Add sensitive-value non-leakage tests in `crates/validator/tests/contract/safe_diagnostics_test.rs`
- [X] T028 [US3] Add nested-error shape and bounded-size tests in `crates/validator/tests/contract/error_tree_bounds_test.rs`

### Implementation for User Story 3

- [X] T029 [P] [US3] Harden message construction to avoid raw secret values in `crates/validator/src/foundation/error.rs`
- [X] T030 [P] [US3] Ensure consistent structured params/help population in `crates/validator/src/combinators/error.rs` and `crates/validator/src/combinators/message.rs`
- [X] T031 [US3] Synchronize diagnostics contract docs with schema in `docs/crates/validator/API.md` and `specs/001-validator-crate-spec/contracts/validation-error-envelope.schema.json`
- [X] T032 [US3] Add operational guidance for safe diagnostics in `docs/crates/validator/SECURITY.md`

**Checkpoint**: US3 diagnostics are actionable, structured, and security-safe.

---

## Phase 6: User Story 4 - Governed Evolution of Validator Surface (Priority: P3)

**Goal**: Establish explicit governance for additive evolution, deprecations, and major-version breaks.

**Independent Test**: Simulate proposal-to-release flow and verify compatibility checks plus migration artifacts are required and complete.

### Tests for User Story 4

- [X] T033 [P] [US4] Add governance compliance tests for additive-only minor changes in `crates/validator/tests/contract/governance_policy_test.rs`
- [X] T034 [US4] Add migration-map presence checks for behavior-significant changes in `crates/validator/tests/contract/migration_requirements_test.rs`

### Implementation for User Story 4

- [X] T035 [P] [US4] Finalize governance policy and change classification in `docs/crates/validator/DECISIONS.md` and `docs/crates/validator/PROPOSALS.md`
- [X] T036 [P] [US4] Finalize release-path requirements and deprecation windows in `docs/crates/validator/MIGRATION.md` and `docs/crates/validator/ROADMAP.md`
- [X] T037 [US4] Add contract test requirements for downstream crates in `docs/crates/validator/INTERACTIONS.md` and `docs/crates/validator/TEST_STRATEGY.md`

**Checkpoint**: US4 governance controls are documented and test-enforced.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Final consistency, quality gates, and release readiness.

- [X] T038 [P] Run validator package tests and benches: `cargo test -p nebula-validator` and `cargo bench -p nebula-validator`
- [ ] T039 [P] Run workspace quality gates and fix issues: `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo check --workspace --all-targets`, `cargo doc --no-deps --workspace`
- [X] T040 Reconcile docs/spec drift across `specs/001-validator-crate-spec/*.md` and `docs/crates/validator/*.md`
- [X] T041 Verify quickstart execution and update commands in `specs/001-validator-crate-spec/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- Setup (Phase 1): no dependencies.
- Foundational (Phase 2): depends on Setup completion; blocks all stories.
- User Stories (Phases 3-6): depend on Foundational completion.
- Polish (Phase 7): depends on completion of selected stories.

### User Story Dependencies

- US1 (P1): starts immediately after Phase 2; no dependency on other stories.
- US2 (P1): starts after Phase 2; can run parallel with US1.
- US3 (P2): starts after Phase 2; can run parallel with US1/US2.
- US4 (P3): starts after Phase 2; best after US1 artifacts exist but remains independently testable.

### Within Each User Story

- Tests first and fail before implementation changes.
- Contract/data fixtures before behavior/documentation finalization.
- Code updates before final story documentation alignment.

## Parallel Opportunities

- Setup: T003 and T004 parallel.
- Foundational: T007 and T008 parallel; T005/T006 can proceed alongside T009/T010 once structure exists.
- US1: T011/T012 parallel; T014/T015 parallel.
- US2: T019/T020/T021 parallel; T022/T023 parallel.
- US3: T026/T027 parallel; T029/T030 parallel.
- US4: T033 parallel with T035/T036.
- Polish: T038 and T039 parallel.

## Parallel Example: User Story 1

```bash
# Tests in parallel
Task: "T011 [US1] compatibility fixtures in crates/validator/tests/contract/compatibility_fixtures_test.rs"
Task: "T012 [US1] typed vs dynamic equivalence in crates/validator/tests/contract/typed_dynamic_equivalence_test.rs"

# Implementation in parallel
Task: "T014 [US1] stable error-code mappings in crates/validator/src/foundation/error.rs"
Task: "T015 [US1] field-path propagation in crates/validator/src/combinators/field.rs and json_field.rs"
```

## Implementation Strategy

### MVP First (US1 only)

1. Complete Phase 1 and Phase 2.
2. Complete Phase 3 (US1).
3. Validate US1 independently via contract fixtures.
4. Ship MVP contract hardening baseline.

### Incremental Delivery

1. Foundation (Phases 1-2).
2. US1 and US2 (both P1), then validate.
3. US3 diagnostics hardening.
4. US4 governance enforcement.
5. Polish and full quality gates.

### Parallel Team Strategy

1. Team aligns on Phases 1-2.
2. Split by stories:
   - Engineer A: US1
   - Engineer B: US2
   - Engineer C: US3
   - Engineer D: US4
3. Integrate in Phase 7 with full gates and drift check.


