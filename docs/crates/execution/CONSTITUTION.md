# nebula-execution Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

A workflow run has lifecycle and state: created, running, paused, cancelling, completed, failed, cancelled, timed_out. The engine and API need a shared model for execution state, result, and flow control so that state machine and persistence are consistent.

**nebula-execution is the execution state machine and runtime flow control model for workflow runs.**

It answers: *What are the states and transitions of a workflow execution, and what result and flow-control types do the engine and runtime use?*

```
Engine creates execution (Created)
    ↓
Engine runs nodes → Running; may Pause, Cancelling, or Complete/Fail/TimedOut
    ↓
ExecutionStatus: Created | Running | Paused | Cancelling | Completed | Failed | Cancelled | TimedOut
    ↓
ExecutionState, NodeExecutionState, NodeOutput/ExecutionOutput, JournalEntry, IdempotencyKey/Manager
```

This is the execution contract: state machine is defined (validated transitions in `transition` module); result and output types are stable; no orchestration or persistence in this crate.

---

## User Stories

### Story 1 — Engine Persists and Queries Execution State (P1)

Engine creates an execution, updates state (Running, Suspended, Completed, Failed), and persists. API or engine later queries by execution_id to get current state and result.

**Acceptance**:
- ExecutionState (or equivalent) has defined variants
- Transitions are documented (e.g. Created → Running → Completed)
- Result type (output, error) is part of execution model
- Persistence is out of scope (engine/storage); execution crate defines shape only

### Story 2 — Suspended Execution Can Be Resumed (P2)

When a node returns Wait (e.g. for webhook), execution becomes Suspended. Resume token or correlation ID is stored; API or webhook later calls "resume" with that token. Execution crate defines the resume handle type, not the storage.

**Acceptance**:
- Suspend state carries resume info (token, condition) as data
- Resume is defined as transition Suspended → Running
- Engine and API own resume protocol; execution crate owns state and result types

### Story 3 — API Returns Execution Status and Result (P2)

GET /api/v1/executions/:id returns state, result (if completed), and optional error. Response shape is derived from execution crate types so that API and engine agree.

**Acceptance**:
- Execution type is Serialize for API response
- State and result are stable fields; additive in minor
- No API-specific logic in execution crate; only types

---

## Core Principles

### I. Execution Crate Is State and Result Model Only

**Execution crate defines ExecutionState, ExecutionResult, and flow-control types. It does not run the engine or persist to storage.**

**Rationale**: Engine owns lifecycle and persistence; execution crate owns the vocabulary. Clear boundary.

**Rules**:
- No dependency on engine or storage for core types
- Persistence and scheduling are in engine/storage
- Optional: execution crate may depend on core (ids, context)

### II. State Machine Is Documented and Deterministic

**Transitions (e.g. Created → Running → Completed) are explicit. No undefined or ambiguous transition.**

**Rationale**: Persistence and API depend on state. Ambiguity would cause bugs and inconsistent UI.

**Rules**:
- State machine diagram or table in docs
- Only defined transitions; invalid transition is error or no-op (documented)
- Resume (e.g. Paused → Running) is a defined transition when implemented

### III. Result and Error Shape Are Stable

**ExecutionResult (output, error, metadata) is serializable and stable for API and storage.**

**Rationale**: API and clients depend on result shape. Breaking it breaks dashboards and integrations.

**Rules**:
- Serialize/Deserialize for result type
- Patch/minor: additive fields only; no removal
- Major: MIGRATION.md for result shape change

### IV. No Orchestration in Execution Crate

**Execution crate does not schedule nodes or run actions. It only defines state and result types.**

**Rationale**: Orchestration is engine's job. Execution is the data model.

**Rules**:
- No engine or runtime dependency for orchestration
- Resume "handle" is data (token); engine interprets it

---

## Production Vision

### The execution model in an n8n-class fleet

In production, every workflow run has an execution record: execution_id, workflow_id, status (Created | Running | Paused | Cancelling | Completed | Failed | Cancelled | TimedOut), per-node state and output, journal, and idempotency keys. Engine updates state and journal; storage persists; API reads and returns. Execution crate is the type authority; engine and API use these types.

```
ExecutionState
    ├── execution_id, workflow_id
    ├── status: ExecutionStatus (8 states; validated transitions)
    ├── node_states: NodeExecutionState (NodeState, attempts, NodeOutput)
    ├── JournalEntry stream (audit)
    └── IdempotencyKey / IdempotencyManager (duplicate detection)
```

From the archives: execution state machine and flow control from architecture-v2. Production vision: ExecutionState, ExecutionStatus, NodeOutput, JournalEntry, IdempotencyKey are implemented and stable; optional resume token for Paused→Running is a gap.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| ExecutionResult/ExecutionSummary type for API | High | Dedicated type for GET /executions/:id response |
| Resume token/handle type (e.g. for Paused→Running) | Medium | For suspend/resume flows |
| State transition validation | Medium | Reject invalid transitions |
| Schema snapshot for API response | Low | Lock JSON shape |

---

## Key Decisions

### D-001: Execution Crate Does Not Persist

**Decision**: Execution crate defines types; engine or storage crate owns persistence.

**Rationale**: Storage abstraction and backend are in storage crate; execution is vocabulary.

**Rejected**: Execution crate writing to DB — would mix model and infrastructure.

### D-002: State Machine in Crate, Transitions in Engine

**Decision**: Execution crate defines state enum and result type; engine performs transitions and persistence.

**Rationale**: Engine owns lifecycle; execution owns vocabulary. Engine calls storage with execution-shaped data.

**Rejected**: Execution crate owning transition logic — would require engine to depend on execution for behavior, or duplicate logic.

### D-003: Resume as Data

**Decision**: Resume token or handle is a value type (e.g. opaque string or struct); engine and API interpret it for resume flow.

**Rationale**: Execution crate stays minimal; resume protocol (how token is created and used) is engine/API concern.

**Rejected**: Resume logic in execution crate — would pull in engine or storage.

---

## Non-Negotiables

1. **Execution crate is state and result model only** — no orchestration or persistence.
2. **State machine is documented and deterministic** — only defined transitions.
3. **Result shape is stable** — serializable; compatible in patch/minor.
4. **Resume is data** — token/handle type; engine/API implement resume protocol.
5. **Breaking state or result = major + MIGRATION.md** — API and engine depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No type or state change.
- **MINOR**: Additive (new optional fields). No removal.
- **MAJOR**: Breaking state or result. Requires MIGRATION.md.
