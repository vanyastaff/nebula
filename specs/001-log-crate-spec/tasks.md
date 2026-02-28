# Tasks: Nebula Log Production Hardening

**Input**: Design documents from `/specs/001-log-crate-spec/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Tests are explicitly included because the spec and constitution require test-first validation for non-trivial behavior.

**Organization**: Tasks are grouped by user story so each story can be implemented and validated independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: User story label (`[US1]`, `[US2]`, `[US3]`, `[US4]`)
- Every task includes an exact file path

## Path Conventions

- Crate scope: `crates/log/`
- Tests: `crates/log/tests/`
- Feature docs/contracts: `specs/001-log-crate-spec/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Prepare workspace checks and feature test scaffolding.

- [X] T001 Create feature task tracker skeleton in specs/001-log-crate-spec/tasks.md
- [X] T002 Create integration test modules for this feature in crates/log/tests/config_precedence.rs
- [X] T003 [P] Create integration test modules for fanout and rolling in crates/log/tests/writer_fanout.rs
- [X] T004 [P] Create integration test modules for hook policy behavior in crates/log/tests/hook_policy.rs
- [X] T005 [P] Create integration test modules for compatibility snapshots in crates/log/tests/config_compatibility.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core contracts and primitives required by all stories.

**⚠️ CRITICAL**: No user story work starts before this phase is complete.

- [X] T006 Define destination failure policy enum and defaults in crates/log/src/config/mod.rs
- [X] T007 [P] Define hook policy config type and defaults in crates/log/src/observability/mod.rs
- [X] T008 [P] Add config validation errors for precedence and policy parsing in crates/log/src/core/error.rs
- [X] T009 Implement config resolution pipeline entrypoint in crates/log/src/builder/mod.rs
- [X] T010 Add feature-level contract assertions for API surface in crates/log/tests/api_contract.rs

**Checkpoint**: Foundation ready; user stories can proceed.

---

## Phase 3: User Story 1 - Reliable Environment Startup (Priority: P1) 🎯 MVP

**Goal**: Deterministic startup behavior across explicit config, env config, and presets.

**Independent Test**: Run config precedence scenarios and invalid filter scenarios in `config_precedence.rs` to verify deterministic resolution and clear failures.

### Tests for User Story 1

- [X] T011 [P] [US1] Write failing test for explicit-over-env precedence in crates/log/tests/config_precedence.rs
- [X] T012 [P] [US1] Write failing test for env-over-preset precedence in crates/log/tests/config_precedence.rs
- [X] T013 [US1] Write failing test for invalid filter startup failure in crates/log/tests/config_precedence.rs

### Implementation for User Story 1

- [X] T014 [US1] Implement precedence merge logic in crates/log/src/config/mod.rs
- [X] T015 [US1] Implement env parsing normalization for precedence inputs in crates/log/src/config/env.rs
- [X] T016 [US1] Wire resolved profile logging during initialization in crates/log/src/lib.rs
- [X] T017 [US1] Add startup failure diagnostics mapping to public errors in crates/log/src/core/error.rs
- [X] T018 [US1] Update precedence contract examples in specs/001-log-crate-spec/contracts/logging-config-contract.md

**Checkpoint**: US1 should be independently testable and shippable as MVP.

---

## Phase 4: User Story 2 - Multi-Destination Delivery with Predictable Failure Policy (Priority: P1)

**Goal**: True fanout across multiple writers with explicit failure policies and size-based rolling.

**Independent Test**: Run multi-writer scenarios with injected sink failure and rolling threshold checks in `writer_fanout.rs`.

### Tests for User Story 2

- [X] T019 [P] [US2] Write failing test for multi-writer fanout delivery in crates/log/tests/writer_fanout.rs
- [X] T020 [P] [US2] Write failing test for best-effort behavior on partial sink failure in crates/log/tests/writer_fanout.rs
- [X] T021 [P] [US2] Write failing test for fail-fast behavior on sink failure in crates/log/tests/writer_fanout.rs
- [X] T022 [US2] Write failing test for size-based rolling activation in crates/log/tests/writer_fanout.rs

### Implementation for User Story 2

- [X] T023 [US2] Implement multi-writer fanout dispatch in crates/log/src/writer.rs
- [X] T024 [US2] Implement failure policy execution paths in crates/log/src/writer.rs
- [X] T025 [US2] Implement size-based rolling strategy in crates/log/src/writer.rs
- [X] T026 [US2] Wire writer policy config into builder pipeline in crates/log/src/builder/mod.rs
- [X] T027 [US2] Add failure-policy documentation and examples in specs/001-log-crate-spec/contracts/observability-delivery-contract.md

**Checkpoint**: US2 should run independently with deterministic delivery/failure behavior.

---

## Phase 5: User Story 3 - Safe Observability Hooks Under Load (Priority: P2)

**Goal**: Keep hook failures isolated and bound hook impact on latency under load.

**Independent Test**: Run panicking and slow hook scenarios in `hook_policy.rs`; ensure emission continuity and bounded behavior.

### Tests for User Story 3

- [X] T028 [P] [US3] Write failing test proving panic isolation across hooks in crates/log/tests/hook_policy.rs
- [X] T029 [P] [US3] Write failing test for bounded hook budget timeout behavior in crates/log/tests/hook_policy.rs
- [X] T030 [US3] Write failing test for shutdown drain bound behavior in crates/log/tests/hook_policy.rs

### Implementation for User Story 3

- [X] T031 [US3] Implement bounded hook execution mode in crates/log/src/observability/registry.rs
- [X] T032 [US3] Preserve panic isolation semantics in bounded and inline modes in crates/log/src/observability/registry.rs
- [X] T033 [US3] Implement hook shutdown budget enforcement in crates/log/src/observability/mod.rs
- [X] T034 [US3] Add hook policy wiring in logger initialization in crates/log/src/builder/mod.rs
- [X] T035 [US3] Document hook hardening behavior in docs/crates/log/RELIABILITY.md

**Checkpoint**: US3 should be independently stable under injected hook faults.

---

## Phase 6: User Story 4 - Upgrade Without Surprise Breakage (Priority: P3)

**Goal**: Preserve additive compatibility and explicit migration guidance.

**Independent Test**: Run compatibility snapshot tests and migration contract checks in `config_compatibility.rs`.

### Tests for User Story 4

- [X] T036 [P] [US4] Write failing snapshot compatibility test for existing config profiles in crates/log/tests/config_compatibility.rs
- [X] T037 [US4] Write failing test for deprecation-window contract expectations in crates/log/tests/config_compatibility.rs

### Implementation for User Story 4

- [X] T038 [US4] Implement config schema version marker and compatibility loader in crates/log/src/config/mod.rs
- [X] T039 [US4] Add compatibility fixtures for prior supported profiles in crates/log/tests/fixtures/config/
- [X] T040 [US4] Update migration guarantees and deprecation contract in docs/crates/log/MIGRATION.md
- [X] T041 [US4] Align API stability notes with new policies in specs/001-log-crate-spec/contracts/api-surface.md
- [X] T042 [US4] Sync schema contract example for versioned config in specs/001-log-crate-spec/contracts/config-schema.json

**Checkpoint**: US4 should guarantee safe minor-version upgrades with explicit migration path.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Final quality gates, performance validation, and end-to-end documentation.

- [X] T043 [P] Add benchmark scenarios for emission/context/hooks in crates/log/benches/log_hot_path.rs
- [X] T044 Define CI regression thresholds for logging benchmarks in .github/workflows/ci.yml
- [X] T045 [P] Update quickstart validation flow with final commands in specs/001-log-crate-spec/quickstart.md
- [X] T046 Run full workspace quality gates and record results in specs/001-log-crate-spec/research.md

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies.
- **Phase 2 (Foundational)**: Depends on Phase 1; blocks all stories.
- **Phase 3-6 (User Stories)**: Depend on Phase 2 completion.
- **Phase 7 (Polish)**: Depends on all selected user stories.

### User Story Dependencies

- **US1 (P1)**: Starts after Phase 2; no dependency on other stories.
- **US2 (P1)**: Starts after Phase 2; no dependency on US1 for independent validation.
- **US3 (P2)**: Starts after Phase 2; may reuse writer/config primitives but remains independently testable.
- **US4 (P3)**: Starts after Phase 2; depends on finalized config behavior from US1/US2 for compatibility fixtures.

### Dependency Graph

- Setup -> Foundational -> {US1, US2, US3} -> US4 -> Polish

### Within Each User Story

- Tests first and failing before implementation.
- Core behavior implementation before documentation updates.
- Story acceptance checks pass before moving to next priority.

## Parallel Opportunities

- **US1**: T011 and T012 parallel.
- **US2**: T019, T020, T021 parallel.
- **US3**: T028 and T029 parallel.
- **US4**: T036 parallel with fixture preparation task T039.
- **Polish**: T043 and T045 parallel.

---

## Parallel Example: User Story 2

```bash
Task: "T019 [US2] Write failing test for multi-writer fanout delivery in crates/log/tests/writer_fanout.rs"
Task: "T020 [US2] Write failing test for best-effort behavior on partial sink failure in crates/log/tests/writer_fanout.rs"
Task: "T021 [US2] Write failing test for fail-fast behavior on sink failure in crates/log/tests/writer_fanout.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 and Phase 2.
2. Complete US1 (Phase 3).
3. Validate US1 independently via `crates/log/tests/config_precedence.rs`.
4. Demo deterministic startup behavior.

### Incremental Delivery

1. Setup + Foundational.
2. Deliver US1 (startup determinism).
3. Deliver US2 (fanout + rolling).
4. Deliver US3 (hook resilience).
5. Deliver US4 (upgrade safety).
6. Finish polish and performance gates.

### Parallel Team Strategy

1. One engineer closes Phase 2.
2. After Phase 2:
   - Engineer A: US1
   - Engineer B: US2
   - Engineer C: US3
3. Engineer D begins US4 once config behavior stabilizes.

## Notes

- `[P]` tasks are safe parallel work on independent files or independent assertions.
- Every user story has explicit independent test criteria.
- Task descriptions are execution-ready and reference concrete repository paths.
