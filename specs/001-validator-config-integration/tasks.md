# Tasks: Validator Integration in Config Crate

**Input**: Design documents from `/specs/001-validator-config-integration/`  
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`

**Tests**: Included. The feature specification explicitly requires independent, contract-level verification.

**Organization**: Tasks are grouped by user story to keep each slice independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on incomplete tasks)
- **[Story]**: User story label (`[US1]`, `[US2]`, `[US3]`)
- Every task includes explicit file path(s)

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Prepare feature scaffolding and integration test entrypoints.

- [X] T001 Create feature task file at `specs/001-validator-config-integration/tasks.md` and align with plan artifacts
- [X] T002 Create contract fixture directory and baseline files in `crates/config/tests/fixtures/compat/` for validator-integration scenarios
- [X] T003 [P] Add validator-integration contract test module wiring in `crates/config/tests/integration_test.rs` and `crates/config/tests/contract/mod.rs`
- [X] T004 [P] Add validator-integration summary section in `docs/crates/config/README.md` and `docs/crates/validator/README.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Shared contract and lifecycle baseline required before user stories.

**⚠️ CRITICAL**: No user story work begins before this phase is complete.

- [X] T005 Define config-validator stable contract baseline in `docs/crates/config/API.md` and `docs/crates/config/INTERACTIONS.md`
- [X] T006 Define cross-crate category governance in `docs/crates/config/DECISIONS.md` and `docs/crates/validator/DECISIONS.md`
- [X] T007 [P] Add fixture schema/readme for validator-integration compatibility in `crates/config/tests/fixtures/compat/README.md`
- [X] T008 [P] Add shared contract helpers for validation outcome assertions in `crates/config/tests/contract/helpers.rs`
- [X] T009 Add migration mapping template for config-validator contract changes in `docs/crates/config/MIGRATION.md` and `docs/crates/validator/MIGRATION.md`
- [X] T010 Add validator-integration quality gates in `docs/crates/config/TEST_STRATEGY.md` and `docs/crates/validator/TEST_STRATEGY.md`

**Checkpoint**: Foundation ready for user story work.

---

## Phase 3: User Story 1 - Safe Activation Gate (Priority: P1) 🎯 MVP

**Goal**: Ensure validator-gated activation/reload and deterministic last-known-good preservation.

**Independent Test**: Submit valid and invalid candidates and verify activation/rejection behavior plus active snapshot retention.

### Tests for User Story 1

- [X] T011 [P] [US1] Add valid-candidate activation contract test in `crates/config/tests/contract/validator_activation_contract_test.rs`
- [X] T012 [P] [US1] Add invalid-candidate reload rejection contract test in `crates/config/tests/contract/validator_reload_rejection_contract_test.rs`
- [X] T013 [P] [US1] Add last-known-good retention contract test in `crates/config/tests/contract/validator_last_known_good_test.rs`
- [X] T014 [US1] Add versioned validator-integration fixture in `crates/config/tests/fixtures/compat/validator_activation_v1.json`

### Implementation for User Story 1

- [X] T015 [P] [US1] Enforce validator gate in build/reload lifecycle in `crates/config/src/core/builder.rs` and `crates/config/src/core/config.rs`
- [X] T016 [P] [US1] Normalize validator invocation flow and error propagation in `crates/config/src/validators/composite.rs` and `crates/config/src/core/traits.rs`
- [X] T017 [US1] Stabilize validation outcome category mapping in `crates/config/src/core/error.rs`
- [X] T018 [US1] Document activation/rejection contract and fallback semantics in `docs/crates/config/API.md` and `docs/crates/config/RELIABILITY.md`

**Checkpoint**: US1 behavior is independently verifiable and production-safe.

---

## Phase 4: User Story 2 - Cross-Crate Contract Consistency (Priority: P2)

**Goal**: Lock compatibility semantics between config and validator across releases.

**Independent Test**: Run compatibility fixtures and governance checks to verify stable categories and required migration mapping.

### Tests for User Story 2

- [X] T019 [P] [US2] Add config-validator category compatibility fixture test in `crates/config/tests/contract/validator_category_compatibility_test.rs`
- [X] T020 [P] [US2] Add additive-minor governance test in `crates/config/tests/contract/validator_governance_policy_test.rs`
- [X] T021 [US2] Add migration mapping presence test in `crates/config/tests/contract/validator_migration_requirements_test.rs`
- [X] T022 [US2] Add cross-crate compatibility fixture in `crates/config/tests/fixtures/compat/validator_contract_v1.json`

### Implementation for User Story 2

- [X] T023 [P] [US2] Align category naming contract between crates in `crates/config/src/core/error.rs` and `crates/validator/src/foundation/error.rs`
- [X] T024 [P] [US2] Add/refresh contract references in `docs/crates/config/API.md` and `docs/crates/validator/API.md`
- [X] T025 [US2] Finalize compatibility governance rules in `docs/crates/config/DECISIONS.md` and `docs/crates/validator/DECISIONS.md`
- [X] T026 [US2] Finalize migration and release-path requirements in `docs/crates/config/MIGRATION.md`, `docs/crates/validator/MIGRATION.md`, and `docs/crates/config/ROADMAP.md`

**Checkpoint**: US2 compatibility governance is test-enforced and documented.

---

## Phase 5: User Story 3 - Operator-Ready Diagnostics and Runbooks (Priority: P3)

**Goal**: Provide actionable, redacted diagnostics and clear runbooks for validation failures.

**Independent Test**: Trigger validation failures and verify diagnostics and runbooks support deterministic operator recovery.

### Tests for User Story 3

- [X] T027 [P] [US3] Add sensitive diagnostics redaction contract test in `crates/config/tests/contract/validator_redaction_contract_test.rs`
- [X] T028 [P] [US3] Add operator-context diagnostics contract test in `crates/config/tests/contract/validator_diagnostics_context_test.rs`
- [X] T029 [US3] Add runbook coverage test for failure/recovery guidance in `crates/config/tests/contract/validator_runbook_requirements_test.rs`

### Implementation for User Story 3

- [X] T030 [P] [US3] Improve redacted validation diagnostics in `crates/config/src/core/error.rs` and `crates/config/src/loaders/env.rs`
- [X] T031 [P] [US3] Document operator runbook for validation failures in `docs/crates/config/RELIABILITY.md` and `docs/crates/config/INTERACTIONS.md`
- [X] T032 [US3] Add downstream consumer validation guidance in `docs/crates/config/INTERACTIONS.md` and `docs/crates/validator/INTERACTIONS.md`
- [X] T033 [US3] Update diagnostics and incident-response strategy in `docs/crates/config/TEST_STRATEGY.md` and `docs/crates/validator/TEST_STRATEGY.md`

**Checkpoint**: US3 diagnostics and runbooks are independently testable and operationally usable.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final validation, consistency checks, and quality gates.

- [X] T034 [P] Run config and validator tests: `cargo test -p nebula-config` and `cargo test -p nebula-validator`
- [X] T035 [P] Run benches/no-run checks: `cargo bench -p nebula-config --no-run` and `cargo bench -p nebula-validator --no-run`
- [ ] T036 [P] Run workspace quality gates: `cargo fmt --all -- --check`, `cargo clippy --workspace -- -D warnings`, `cargo check --workspace --all-targets`, `cargo doc --no-deps --workspace`
- [X] T037 Reconcile docs/spec drift across `specs/001-validator-config-integration/*.md`, `docs/crates/config/*.md`, and `docs/crates/validator/*.md`
- [X] T038 Verify quickstart flow and update `specs/001-validator-config-integration/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- Setup (Phase 1): no dependencies.
- Foundational (Phase 2): depends on Setup and blocks all user stories.
- User Stories (Phases 3-5): depend on Foundational.
- Polish (Phase 6): depends on selected story completion.

### User Story Dependencies

- US1 (P1): starts immediately after Phase 2; no dependency on other stories.
- US2 (P2): starts after Phase 2; can run in parallel with US1.
- US3 (P3): starts after Phase 2; can run in parallel with US1/US2.

### Within Each User Story

- Tests first, then implementation.
- Fixture/contracts before documentation closure.
- Lifecycle behavior updates before governance finalization.

## Parallel Opportunities

- Setup: T003, T004.
- Foundational: T007, T008.
- US1: T011/T012/T013 and T015/T016.
- US2: T019/T020 and T023/T024.
- US3: T027/T028 and T030/T031.
- Polish: T034/T035/T036.

---

## Parallel Example: User Story 1

```bash
# Parallel tests
Task: "T011 [US1] validator activation contract test in crates/config/tests/contract/validator_activation_contract_test.rs"
Task: "T012 [US1] reload rejection contract test in crates/config/tests/contract/validator_reload_rejection_contract_test.rs"

# Parallel implementation
Task: "T015 [US1] validator gate lifecycle updates in crates/config/src/core/builder.rs and crates/config/src/core/config.rs"
Task: "T016 [US1] validator invocation flow updates in crates/config/src/validators/composite.rs and crates/config/src/core/traits.rs"
```

---

## Implementation Strategy

### MVP First (US1 only)

1. Complete Phases 1-2.
2. Complete US1 (Phase 3).
3. Validate activation/rejection and last-known-good tests.
4. Ship validator-gated activation MVP.

### Incremental Delivery

1. Foundation (Phases 1-2).
2. US1 activation safety.
3. US2 compatibility governance.
4. US3 diagnostics and runbooks.
5. Polish with full quality gates.

### Parallel Team Strategy

1. Team aligns on Setup + Foundational.
2. Split by story:
   - Engineer A: US1 (activation gate/reload safety)
   - Engineer B: US2 (cross-crate compatibility governance)
   - Engineer C: US3 (diagnostics and runbooks)
3. Integrate in Phase 6.
