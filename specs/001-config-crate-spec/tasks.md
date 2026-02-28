# Tasks: Config Contract Hardening

**Input**: Design documents from `/specs/001-config-crate-spec/`  
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: Included. Feature spec explicitly requires independent test criteria and contract-level compatibility checks.

**Organization**: Tasks are grouped by user story to keep each slice independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on incomplete task)
- **[Story]**: User story label (`[US1]`, `[US2]`, `[US3]`, `[US4]`)
- Every task includes explicit file path(s)

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Prepare contract-test scaffolding and documentation anchors.

- [X] T001 Create config contract test directory and fixture folders in `crates/config/tests/contract/` and `crates/config/tests/fixtures/compat/`
- [X] T002 Add contract test module wiring in `crates/config/tests/integration_test.rs` and `crates/config/tests/mod.rs` (create if missing)
- [X] T003 [P] Add feature task-phase references in `specs/001-config-crate-spec/quickstart.md`
- [X] T004 [P] Add config contract hardening summary section in `docs/crates/config/README.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared contract and governance baseline required before all stories.

**⚠️ CRITICAL**: No story work begins before this phase is complete.

- [X] T005 Define canonical precedence and merge contract baseline in `docs/crates/config/API.md` and `docs/crates/config/DECISIONS.md`
- [X] T006 Define path access and typed retrieval error-category contract in `docs/crates/config/API.md` and `docs/crates/config/INTERACTIONS.md`
- [X] T007 [P] Add compatibility fixture format documentation in `crates/config/tests/fixtures/compat/README.md`
- [X] T008 [P] Add shared contract assertion helpers in `crates/config/tests/contract/helpers.rs`
- [X] T009 Add migration mapping template for precedence/path/validation changes in `docs/crates/config/MIGRATION.md`
- [X] T010 Add validator-focused CI quality gate section in `docs/crates/config/TEST_STRATEGY.md`

**Checkpoint**: Foundation ready for user story work.

---

## Phase 3: User Story 1 - Deterministic Layered Configuration Outcomes (Priority: P1) 🎯 MVP

**Goal**: Guarantee deterministic layered source resolution (including `env`) across runs and minor releases.

**Independent Test**: Execute precedence fixtures combining defaults/file/env/inline and verify stable resolved outputs and error behavior.

### Tests for User Story 1

- [X] T011 [P] [US1] Add precedence matrix contract tests in `crates/config/tests/contract/precedence_matrix_contract_test.rs`
- [X] T012 [P] [US1] Add merge determinism repeatability tests in `crates/config/tests/contract/merge_determinism_test.rs`
- [X] T013 [P] [US1] Add `env` override contract tests in `crates/config/tests/contract/env_precedence_contract_test.rs`
- [X] T014 [US1] Add versioned precedence fixtures in `crates/config/tests/fixtures/compat/precedence_v1.json`

### Implementation for User Story 1

- [X] T015 [P] [US1] Normalize source precedence behavior in `crates/config/src/core/builder.rs` and `crates/config/src/loaders/composite.rs`
- [X] T016 [P] [US1] Align environment loader key mapping/override behavior in `crates/config/src/loaders/env.rs`
- [X] T017 [US1] Document layered precedence examples (`defaults < file < env < inline`) in `docs/crates/config/API.md`
- [X] T018 [US1] Document optional-source failure behavior and diagnostics in `docs/crates/config/INTERACTIONS.md`

**Checkpoint**: US1 precedence and deterministic merge are independently verifiable.

---

## Phase 4: User Story 2 - Safe Validation and Reload Activation (Priority: P1)

**Goal**: Ensure invalid reload candidates are rejected and active state remains last-known-good.

**Independent Test**: Run reload scenarios where invalid candidates are submitted after valid activation and verify atomic rejection/retention behavior.

### Tests for User Story 2

- [X] T019 [P] [US2] Add invalid-reload rejection tests in `crates/config/tests/contract/reload_rejection_contract_test.rs`
- [X] T020 [P] [US2] Add last-known-good preservation tests in `crates/config/tests/contract/last_known_good_preservation_test.rs`
- [X] T021 [P] [US2] Add activation atomicity tests in `crates/config/tests/contract/activation_atomicity_test.rs`

### Implementation for User Story 2

- [X] T022 [P] [US2] Align reload lifecycle state transitions in `crates/config/src/core/config.rs` and `crates/config/src/core/builder.rs`
- [X] T023 [P] [US2] Ensure validator-gated activation flow in `crates/config/src/validators/composite.rs` and `crates/config/src/core/traits.rs`
- [X] T024 [US2] Document reload failure runbook and fallback semantics in `docs/crates/config/RELIABILITY.md`
- [X] T025 [US2] Update reload/backoff and failure propagation guidance in `docs/crates/config/TEST_STRATEGY.md`

**Checkpoint**: US2 reload safety behavior is independently verified.

---

## Phase 5: User Story 3 - Stable Typed Access and Path Contracts (Priority: P2)

**Goal**: Stabilize path-based typed retrieval semantics and deterministic error categories.

**Independent Test**: Execute typed path fixtures across versions for success, missing path, and type mismatch outcomes.

### Tests for User Story 3

- [X] T026 [P] [US3] Add typed retrieval compatibility tests in `crates/config/tests/contract/typed_access_compatibility_test.rs`
- [X] T027 [P] [US3] Add missing-path and type-mismatch category tests in `crates/config/tests/contract/path_error_categories_test.rs`
- [X] T028 [US3] Add path contract fixtures in `crates/config/tests/fixtures/compat/path_contract_v1.json`

### Implementation for User Story 3

- [X] T029 [P] [US3] Align `get<T>` error category mapping in `crates/config/src/core/config.rs` and `crates/config/src/core/error.rs`
- [X] T030 [P] [US3] Harden path traversal consistency in `crates/config/src/core/config.rs` and `crates/config/src/core/source.rs`
- [X] T031 [US3] Synchronize path/typed retrieval contract docs in `docs/crates/config/API.md` and `docs/crates/config/INTERACTIONS.md`
- [X] T032 [US3] Add contract schema reference docs in `specs/001-config-crate-spec/contracts/config-error-envelope.schema.json` and `docs/crates/config/API.md`

**Checkpoint**: US3 typed path contract is independently testable and stable.

---

## Phase 6: User Story 4 - Governed Evolution and Migration Clarity (Priority: P3)

**Goal**: Enforce additive minor evolution and explicit migration for breaking contract changes.

**Independent Test**: Validate governance and migration checks against release-readiness fixtures.

### Tests for User Story 4

- [X] T033 [P] [US4] Add governance compliance tests for additive minor changes in `crates/config/tests/contract/governance_policy_test.rs`
- [X] T034 [US4] Add migration mapping presence checks in `crates/config/tests/contract/migration_requirements_test.rs`

### Implementation for User Story 4

- [X] T035 [P] [US4] Finalize compatibility governance decisions in `docs/crates/config/DECISIONS.md` and `docs/crates/config/PROPOSALS.md`
- [X] T036 [P] [US4] Finalize migration/release-path requirements in `docs/crates/config/MIGRATION.md` and `docs/crates/config/ROADMAP.md`
- [X] T037 [US4] Add downstream consumer contract-test requirements in `docs/crates/config/INTERACTIONS.md` and `docs/crates/config/TEST_STRATEGY.md`

**Checkpoint**: US4 governance controls and migration requirements are test-enforced.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Final validation, performance checks, and consistency.

- [X] T038 [P] Run config crate tests and benches: `cargo test -p nebula-config` and `cargo bench -p nebula-config --no-run`
- [ ] T039 [P] Run workspace quality gates: `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo check --workspace --all-targets`, `cargo doc --no-deps --workspace`
- [X] T040 Reconcile docs/spec drift across `specs/001-config-crate-spec/*.md` and `docs/crates/config/*.md`
- [X] T041 Verify quickstart commands and update `specs/001-config-crate-spec/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- Setup (Phase 1): no dependencies.
- Foundational (Phase 2): depends on Setup; blocks all user stories.
- User Stories (Phases 3-6): depend on Foundational.
- Polish (Phase 7): depends on selected story completion.

### User Story Dependencies

- US1 (P1): starts immediately after Phase 2; no dependency on other stories.
- US2 (P1): starts after Phase 2; can run in parallel with US1.
- US3 (P2): starts after Phase 2; can run in parallel with US1/US2.
- US4 (P3): starts after Phase 2; benefits from US1 artifacts but remains independently testable.

### Within Each User Story

- Tests first, then implementation.
- Fixtures/contracts before docs finalization.
- Code changes before governance/doc closure for each story.

## Parallel Opportunities

- Setup: T003 and T004.
- Foundational: T007 and T008.
- US1: T011/T012/T013 in parallel; T015/T016 in parallel.
- US2: T019/T020/T021 in parallel; T022/T023 in parallel.
- US3: T026/T027 in parallel; T029/T030 in parallel.
- US4: T033 parallel with T035/T036.
- Polish: T038 and T039 parallel.

## Parallel Example: User Story 1

```bash
# Parallel tests
Task: "T011 [US1] precedence matrix in crates/config/tests/contract/precedence_matrix_contract_test.rs"
Task: "T013 [US1] env precedence in crates/config/tests/contract/env_precedence_contract_test.rs"

# Parallel implementation
Task: "T015 [US1] precedence normalization in crates/config/src/core/builder.rs and loaders/composite.rs"
Task: "T016 [US1] env loader override behavior in crates/config/src/loaders/env.rs"
```

## Implementation Strategy

### MVP First (US1 only)

1. Complete Phases 1-2.
2. Complete US1 (Phase 3).
3. Validate precedence + `env` override compatibility fixtures.
4. Ship deterministic-resolution MVP.

### Incremental Delivery

1. Foundation (Phases 1-2).
2. US1 + US2 (both P1) and validate.
3. US3 typed path compatibility.
4. US4 governance and migration hardening.
5. Polish with full quality gates.

### Parallel Team Strategy

1. Team aligns on Setup + Foundational.
2. Split by story:
   - Engineer A: US1 (precedence/env)
   - Engineer B: US2 (reload safety)
   - Engineer C: US3 (typed access/path)
   - Engineer D: US4 (governance/migration)
3. Integrate in Phase 7.
