# Tasks: nebula-engine

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix `ENG`

---

## Phase 1: Contract and State Integration 🔄

**Goal**: Wire execution state to nebula-storage; stabilize execute_workflow contract and runtime handoff.

**Prerequisite**: `nebula-storage` Phase 1 (STG-T001–T005) must be complete first.

- [ ] ENG-T001 Implement state store integration — persist `ExecutionState` via `nebula-storage` in `src/state.rs`
- [ ] ENG-T002 Implement state reload on engine restart — query execution by ID from storage
- [ ] ENG-T003 Write contract tests for `execute_workflow` in `tests/contracts.rs` — verify input/output types
- [ ] ENG-T004 [P] Write contract tests for `ExecutionResult` — serialization + deserialization roundtrip
- [ ] ENG-T005 Write contract test for runtime handoff — engine schedules, runtime executes, result returned
- [ ] ENG-T006 Stabilize `ExecutionContext` struct and lifecycle methods in `src/context.rs`
- [ ] ENG-T007 Update API.md with `ExecutionContext` and `execute_workflow` contracts

**Checkpoint**: Single-node workflow runs persist state; engine can resume/query execution by ID; no actions executed in engine itself.

---

## Phase 2: Runtime Hardening ⬜

**Goal**: Trigger lifecycle, backpressure integration, deterministic scheduling tests.

- [ ] ENG-T008 Design trigger lifecycle contract — register/unregister/start/stop API in `src/trigger.rs`
- [ ] ENG-T009 Integrate backpressure via `nebula-system` pressure events in engine scheduler
- [ ] ENG-T010 Implement configurable admission control — reject vs queue behavior under load
- [ ] ENG-T011 [P] Write deterministic scheduling tests — verify DAG order is stable across runs in `tests/scheduling.rs`
- [ ] ENG-T012 [P] Write wait/suspend path tests — verify execution pauses and resumes correctly
- [ ] ENG-T013 Document scheduling order invariants in API.md

**Checkpoint**: Scheduling order defined by DAG; trigger lifecycle operational; admission control configurable and observable.

---

## Phase 3: Observability and Operations ⬜

**Goal**: EventBus metrics for dashboards; optional execution idempotency; operational hooks.

- [ ] ENG-T014 Wire `EventBus` emissions for execution lifecycle events (started, node-complete, finished, failed)
- [ ] ENG-T015 [P] Add execution duration metrics via `nebula-telemetry`
- [ ] ENG-T016 [P] Add node-level aggregate metrics for telemetry dashboards
- [ ] ENG-T017 Implement optional idempotency keys for execution — integrate with `nebula-idempotency` if available
- [ ] ENG-T018 Verify fire-and-forget event contract — event delivery never blocks execution path
- [ ] ENG-T019 Document idempotency key format in API.md

**Checkpoint**: Execution events emitted fire-and-forget; metrics visible in telemetry; idempotency key format stable.

---

## Phase 4: Ecosystem and DX ⬜

**Goal**: Clear API/worker contract documentation; migration path; composition cookbook.

- [ ] ENG-T020 Document API/worker contract — how to start, cancel, and query executions in API.md
- [ ] ENG-T021 [P] Write MIGRATION.md entry for any execution/context contract change policy
- [ ] ENG-T022 Write cookbook example: engine + runtime + storage composition in `docs/crates/engine/examples/`
- [ ] ENG-T023 [P] Write cookbook example: multi-node DAG workflow execution end-to-end

**Checkpoint**: Single documented composition pattern; API/worker contract in API.md; cookbook examples runnable.

---

## Dependencies & Execution Order

- Phase 1 requires `nebula-storage` Phase 1 (Postgres) to be complete first
- Phase 1 → Phase 2 → Phase 3 → Phase 4 (sequential)
- [P] tasks within phases can run in parallel

## Verification (after all phases)

- [ ] `cargo check -p nebula-engine`
- [ ] `cargo test -p nebula-engine`
- [ ] `cargo clippy -p nebula-engine -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-engine`
- [ ] End-to-end: single-node workflow runs with persistent state
