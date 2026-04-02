# Tasks: nebula-workflow

**Roadmap**: [ROADMAP.md](ROADMAP.md) | **Plan**: [PLAN.md](PLAN.md)

## Legend

- `[P]` -- Can run in parallel with other `[P]` tasks in same phase
- `WFL-TXXX` -- Task ID
- `->` -- Depends on previous task

---

## Phase 1: Contract and Schema Baseline

**Goal**: Establish formal workflow definition and DAG API

- [x] WFL-T001 [P] Define `WorkflowDefinition` struct with all required fields (`src/definition.rs`)
- [x] WFL-T002 [P] Define node and connection types (`src/node.rs`, `src/connection.rs`)
- [x] WFL-T003 [P] Implement DAG graph representation using `petgraph` (`src/graph.rs`)
- [x] WFL-T004 Implement `validate_workflow()` with cycle detection and reference validation (-> T001, T002, T003)
- [x] WFL-T005 Define structured `WorkflowError` enum with all validation error variants
- [x] WFL-T006 [P] Implement workflow builder API (`src/builder.rs`)
- [x] WFL-T007 Write unit tests for all validation paths (cycle, refs, empty, duplicates)
- [x] WFL-T008 Align docs (ARCHITECTURE.md, API.md) with current types

**Checkpoint**: Engine and API depend on workflow crate only; all validation paths tested.

---

## Phase 2: Schema Stability and Compatibility

**Goal**: Guarantee serialized schema stability via snapshot tests

- [ ] WFL-T009 [P] Create JSON fixture files for `WorkflowDefinition` serialized form (`tests/fixtures/`)
- [ ] WFL-T010 [P] Create JSON fixture files for node and connection serialized forms (`tests/fixtures/`)
- [ ] WFL-T011 Write roundtrip snapshot tests asserting serialize/deserialize stability (`tests/schema_snapshots.rs`) (-> T009, T010)
- [ ] WFL-T012 Add CI enforcement for snapshot tests (-> T011)
- [ ] WFL-T013 [P] Add version field to `WorkflowDefinition` with compatibility policy
- [ ] WFL-T014 Document serialized form in `API.md` (-> T013)
- [ ] WFL-T015 Document compatibility rules in `MIGRATION.md` or `CONSTITUTION.md` (-> T013)
- [ ] WFL-T016 Add guard test that fails on unexpected new fields in serialized output (-> T011)

**Checkpoint**: Fixtures in repo; CI checks roundtrip; no breaking change without major + MIGRATION.

---

## Phase 3: Validation and Integrations

**Goal**: Composable validation with field-path errors for API responses

- [ ] WFL-T017 [P] Evaluate `nebula-validator` integration approach and define adapter interface
- [ ] WFL-T018 Implement optional `nebula-validator` composable rules for workflow validation (-> T017)
- [ ] WFL-T019 [P] Enhance validation errors with field path information for API 400 responses
- [ ] WFL-T020 Write integration tests ensuring API and engine use same validation entry point (-> T018, T019)
- [ ] WFL-T021 Audit `WorkflowDefinition` to ensure no UI-only or execution-only fields are present
- [ ] WFL-T022 Document validation contract and entry points (-> T020)

**Checkpoint**: Validation contract documented; API and engine share validation; definition is design-time only.

---

## Phase 4: Ecosystem and DX

**Goal**: Great workflow authoring experience

- [ ] WFL-T023 [P] Improve builder API ergonomics for API and CLI workflow creation
- [ ] WFL-T024 [P] Create migration tooling or guidance for schema version bumps
- [ ] WFL-T025 Write operator guidance: when to validate, where errors surface (-> T023)
- [ ] WFL-T026 Add end-to-end examples for common workflow patterns (-> T023)
- [ ] WFL-T027 Ensure builder API and raw struct usage converge on same validation path (-> T023)

**Checkpoint**: Clear path for workflow authoring; low-friction adoption for API/UI.

---

## Dependencies & Execution Order

- Phase 1 -> Phase 2 -> Phase 3 -> Phase 4
- Phase 1 is complete
- [P] tasks within a phase can run in parallel
- Phase 3 can begin before Phase 2 is fully complete (T017, T019 are independent)

## Verification (after all phases)

- [ ] `cargo check -p nebula-workflow --all-features`
- [ ] `cargo test -p nebula-workflow`
- [ ] `cargo clippy -p nebula-workflow -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-workflow`
