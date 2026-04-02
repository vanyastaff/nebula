# Tasks: nebula-execution

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` -- Can run in parallel with other `[P]` tasks in same phase
- `EXC-TXXX` -- Task ID
- `->` -- Depends on previous task

---

## Phase 1: Contract and State Machine

**Goal**: Define all execution types with validated transitions and serde roundtrip

- [x] EXC-T001 [P] Define `ExecutionStatus` enum with all valid states (`src/status.rs`)
- [x] EXC-T002 [P] Define `ExecutionState` and `NodeExecutionState` structs (`src/state.rs`)
- [x] EXC-T003 [P] Implement validated state transitions with transition matrix
- [x] EXC-T004 [P] Define `ExecutionPlan` type for engine plan building (`src/plan.rs`)
- [x] EXC-T005 [P] Define `JournalEntry` for audit trail (`src/journal.rs`)
- [x] EXC-T006 [P] Define `NodeOutput`/`ExecutionOutput` and `NodeAttempt` types (`src/output.rs`)
- [x] EXC-T007 [P] Implement `IdempotencyKey` and `IdempotencyManager` (`src/idempotency.rs`)
- [x] EXC-T008 [P] Define `ExecutionError` enum with structured error variants
- [x] EXC-T009 Write unit tests for all transition matrix paths (-> T001, T002, T003)
- [x] EXC-T010 Write serde roundtrip tests for state and output types (-> T002, T006)

**Checkpoint**: All transition tests pass; serde roundtrip verified; engine can build plan and apply transitions.

---

## Phase 2: API and Schema Stability

**Goal**: Guarantee serialized form stability for API compatibility

- [ ] EXC-T011 [P] Create JSON fixture file for `ExecutionState` serialized form (`tests/fixtures/execution_state.json`)
- [ ] EXC-T012 [P] Create JSON fixture file for `NodeOutput` serialized form (`tests/fixtures/node_output.json`)
- [ ] EXC-T013 [P] Create JSON fixture file for `JournalEntry` serialized form (`tests/fixtures/journal_entry.json`)
- [ ] EXC-T014 Write roundtrip snapshot tests for all fixture types (`tests/schema_snapshots.rs`) (-> T011, T012, T013)
- [ ] EXC-T015 Add CI enforcement for snapshot tests (-> T014)
- [ ] EXC-T016 Document serialized form of all public types in `API.md`
- [ ] EXC-T017 [P] Evaluate and optionally define resume token type for suspend/resume scenarios

**Checkpoint**: Fixtures in repo; CI checks roundtrip; API contract tests use execution types.

---

## Phase 3: Idempotency and Resume

**Goal**: Align with nebula-idempotency for persistent key store

- [ ] EXC-T018 Document current `IdempotencyKey` format and stability guarantees
- [ ] EXC-T019 Align `IdempotencyKey` format with `nebula-idempotency` persistent store (-> T018)
- [ ] EXC-T020 Coordinate key type ownership between execution and idempotency crates (-> T019)
- [ ] EXC-T021 [P] Define `DuplicateIdempotencyKey` error semantics and document
- [ ] EXC-T022 [P] Evaluate resume token design for Paused/wait-for-webhook states
- [ ] EXC-T023 Implement resume token or Resume variant in state if accepted (-> T022)
- [ ] EXC-T024 Write tests for idempotency key persistence roundtrip (-> T019)
- [ ] EXC-T025 Document resume path if implemented (-> T023)

**Checkpoint**: Idempotency key format documented and stable; engine can persist keys; resume path documented.

---

## Phase 4: Observability and Operational Hooks

**Goal**: State and journal sufficient for audit, metrics, and dashboards

- [ ] EXC-T026 [P] Verify `JournalEntry` fields are sufficient for audit logging
- [ ] EXC-T027 [P] Verify state transitions emit enough information for metrics derivation
- [ ] EXC-T028 Add optional execution duration field to `ExecutionState` or `JournalEntry` (-> T026)
- [ ] EXC-T029 Add optional node duration aggregates for dashboard consumption (-> T027)
- [ ] EXC-T030 Write integration test: derive metrics from state and journal entries (-> T028, T029)
- [ ] EXC-T031 Verify no breaking changes introduced in this phase (-> T030)

**Checkpoint**: Engine and telemetry can derive metrics from state and journal; no breaking changes.

---

## Dependencies & Execution Order

- Phase 1 -> Phase 2 -> Phase 3 -> Phase 4
- Phase 1 is complete
- [P] tasks within a phase can run in parallel
- Phase 3 depends on Phase 2 for schema stability guarantees
- Phase 4 can partially overlap with Phase 3 (T026, T027 are independent)

## Verification (after all phases)

- [ ] `cargo check -p nebula-execution --all-features`
- [ ] `cargo test -p nebula-execution`
- [ ] `cargo clippy -p nebula-execution -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-execution`
