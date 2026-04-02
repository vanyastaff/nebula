# Implementation Plan: nebula-engine

**Crate**: `nebula-engine` | **Path**: `crates/engine` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

The engine crate owns workflow execution orchestration — DAG scheduling, execution lifecycle (create → schedule → persist → emit events), and handoff to runtime/sandbox. The engine does not implement actions; all execution flows through runtime. Current focus is state store integration and contract stabilization.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio
**Key Dependencies**: `nebula-core`, `nebula-workflow`, `nebula-execution`, `nebula-storage`, `nebula-action`, `nebula-system`, `nebula-telemetry`
**Testing**: `cargo test -p nebula-engine`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 1: Contract and State Integration | 🔄 In Progress | Requires storage backend (STG-Phase 1 must be done first) |
| Phase 2: Runtime Hardening | ⬜ Planned | Trigger lifecycle, backpressure, deterministic scheduling |
| Phase 3: Observability and Operations | ⬜ Planned | EventBus metrics, idempotency, operational hooks |
| Phase 4: Ecosystem and DX | ⬜ Planned | API/worker contract, cookbook examples |

## Phase Details

### Phase 1: Contract and State Integration

**Goal**: Persist and reload execution state; stabilize `execute_workflow`, `ExecutionResult`, and lifecycle handoff to runtime.

**Deliverables**:
- Full state store backend integration via `nebula-storage`
- Contract tests for `execute_workflow`, `ExecutionResult`, runtime handoff
- `ExecutionContext` and execution lifecycle stable and documented

**Exit Criteria**:
- Single-node and multi-node runs persist state; engine can resume/query execution by ID
- No action implementation in engine; all execution via runtime/sandbox

**Risks**:
- State store API drift between engine, storage, and execution crates

**Dependencies**: `nebula-storage` Phase 1 (Postgres backend) must be complete

### Phase 2: Runtime Hardening

**Goal**: Trigger lifecycle, backpressure, deterministic scheduling, wait/suspend paths.

**Deliverables**:
- Trigger lifecycle: register/unregister/start/stop (trigger types in nebula-action)
- Backpressure + admission control via `nebula-system` pressure events
- Deterministic scheduling and wait/suspend paths documented and tested

**Exit Criteria**:
- Scheduling order defined by DAG + explicit wait/trigger; no hidden non-determinism
- Under pressure, engine behavior (reject vs queue) configurable and observable

**Risks**:
- Trigger lifecycle coupling engine to runtime/action in complex ways
- Admission policy too strict or too loose for production load

**Dependencies**: Phase 1

### Phase 3: Observability and Operations

**Goal**: EventBus + metrics for dashboards; optional idempotency; operational hooks.

**Deliverables**:
- EventBus and metrics sufficient for "list executions", "execution detail", dashboards
- Optional idempotency/deduplication for execution keys (align with nebula-idempotency)
- Execution duration and node-level aggregates for telemetry

**Exit Criteria**:
- Fire-and-forget event contract preserved (no blocking on event delivery)
- Idempotency key format documented and stable

**Risks**:
- Observability that blocks execution path

**Dependencies**: Phase 2

### Phase 4: Ecosystem and DX

**Goal**: Clear API/worker contract, migration path, cookbook.

**Deliverables**:
- Contract for API/worker: how to start, cancel, and query executions
- `MIGRATION.md` for any execution/context contract change
- Cookbook: engine + runtime + storage composition examples

**Exit Criteria**:
- Single documented composition pattern
- Breaking execution/context contract = major version + MIGRATION

**Dependencies**: Phase 3

## Inter-Crate Dependencies

- **Depends on**: `nebula-core`, `nebula-workflow`, `nebula-execution`, `nebula-storage` (state persistence), `nebula-action` (trigger types), `nebula-system` (pressure events)
- **Depended by**: `nebula-api`, `nebula-runtime` (receives scheduling decisions), `nebula-sdk`

## Verification

- [ ] `cargo check -p nebula-engine`
- [ ] `cargo test -p nebula-engine`
- [ ] `cargo clippy -p nebula-engine -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-engine`
