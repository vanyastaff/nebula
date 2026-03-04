# Implementation Plan: nebula-execution

**Crate**: `nebula-execution` | **Path**: `crates/execution` | **Roadmap**: [ROADMAP.md](ROADMAP.md)

## Summary

`nebula-execution` defines the execution state machine, journal entries, idempotency keys, and execution planning types. It is strictly a state and model crate -- orchestration and action execution live in engine/runtime. Phase 1 (contract and state machine) is complete. Current focus is on schema stability and idempotency alignment.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (via `tokio-util` for cancellation)
**Key Dependencies**: `nebula-core`, `nebula-workflow`, `nebula-action`, `serde`, `serde_json`, `chrono`, `thiserror`, `parking_lot`, `tokio-util`
**Testing**: `cargo test -p nebula-execution`

## Current Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Contract and State Machine | Done | ExecutionStatus, state transitions, plans, journals |
| Phase 2: API and Schema Stability | Planned | JSON fixtures, snapshot tests |
| Phase 3: Idempotency and Resume | Planned | Align with nebula-idempotency, resume tokens |
| Phase 4: Observability and Operational Hooks | Planned | Duration aggregates, metrics derivation |

## Phase Details

### Phase 1: Contract and State Machine

**Goal**: Define all execution types with validated transitions and serde roundtrip.

**Deliverables**:
- `ExecutionStatus`, `ExecutionState`, `NodeExecutionState`
- Validated transitions
- `ExecutionPlan`, `JournalEntry`
- `NodeOutput`/`ExecutionOutput`, `NodeAttempt`
- `IdempotencyKey`/`IdempotencyManager`
- `ExecutionError`
- Unit tests for transitions and serde

**Exit Criteria**:
- All transition tests pass
- Serde roundtrip for state and output
- Engine can build plan and apply transitions

### Phase 2: API and Schema Stability

**Goal**: Guarantee serialized form stability for API compatibility.

**Deliverables**:
- Formal schema snapshot (JSON fixtures) for `ExecutionState`, `NodeOutput`, `JournalEntry`
- Document serialized form in API.md
- Optional: resume token type for suspend/resume

**Exit Criteria**:
- Fixtures in repo; CI checks public types roundtrip
- API contract tests use execution types

**Risks**:
- Schema drift if engine or API adds fields without going through execution crate

### Phase 3: Idempotency and Resume

**Goal**: Align idempotency key format with persistent key store; support resume.

**Deliverables**:
- Align with `nebula-idempotency` for persistent key store
- `IdempotencyKey` format stability
- Optional: resume token for Paused/wait-for-webhook states
- Optional: Resume variant or field in state

**Exit Criteria**:
- Idempotency key format documented and unchanged
- Engine or idempotency crate can persist keys
- Resume path documented if implemented

**Risks**:
- Idempotency crate may want to own key type; coordination on format and `DuplicateIdempotencyKey` semantics

### Phase 4: Observability and Operational Hooks

**Goal**: State and journal sufficient for audit, metrics, and dashboards.

**Deliverables**:
- `JournalEntry` and state transitions sufficient for audit and metrics
- Optional: execution duration, node duration aggregates in state or journal

**Exit Criteria**:
- Engine and telemetry can derive metrics from state and journal
- No breaking change to execution crate

**Risks**:
- None if limited to existing types

## Dependencies

| Depends On | Why |
|-----------|-----|
| nebula-core | Identifiers, scope types |
| nebula-workflow | Workflow definition types for execution planning |
| nebula-action | Action types for node execution state |

| Depended By | Why |
|------------|-----|
| nebula-engine | Execution state management, plan building |
| nebula-runtime | Execution lifecycle |
| nebula-api | Execution status endpoints |

## Verification

- [ ] `cargo check -p nebula-execution`
- [ ] `cargo test -p nebula-execution`
- [ ] `cargo clippy -p nebula-execution -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-execution`
