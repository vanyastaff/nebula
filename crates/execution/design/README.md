# nebula-execution

Execution state machine, journals, idempotency, and planning for the Nebula workflow engine. This crate is **state and model only**: it defines execution-time types and validated transitions; it does **not** perform orchestration, action execution, or persistence.

## Scope

- **In scope:**
  - `ExecutionStatus` — execution-level state machine (8 states: Created, Running, Paused, Cancelling, Completed, Failed, Cancelled, TimedOut)
  - `ExecutionState` and `NodeExecutionState` — persistent state tracking with validated transitions
  - `ExecutionPlan` — pre-computed parallel execution schedule from workflow definition
  - `ExecutionContext` — execution-scoped context (execution_id, budget) passed to engine/runtime
  - `JournalEntry` — audit log of execution events
  - `NodeOutput` / `ExecutionOutput` — node output data (inline JSON or blob ref) with metadata
  - `NodeAttempt` — individual attempt tracking with idempotency key
  - `IdempotencyKey` and `IdempotencyManager` — exactly-once guarantees (in-memory)
  - Transition validation: `validate_execution_transition`, `validate_node_transition` (via `nebula_workflow::NodeState`)

- **Out of scope:**
  - Workflow scheduling and DAG orchestration (engine)
  - Persistence of execution state (engine/storage)
  - Action execution (runtime)

## Crate boundaries

| Crate | Responsibility |
|-------|-----------------|
| **nebula-execution** | State and model only: execution state machine, journals, idempotency, plan/output types; no orchestration, no action execution, no persistence. |
| **nebula-action** | Action contract: traits, metadata, ports, result/output/error, context; no scheduling, no sandbox internals. |
| **nebula-engine** (planned) | DAG orchestration: builds state/plan, applies transitions, persists state and journal; delegates node execution to runtime. |
| **nebula-runtime** (planned) | Action execution: runs actions (StatelessAction/StatefulAction/etc.), idempotency check, sandbox/capabilities. |

## Current State

- **Maturity:** Implemented and tested; state machine, plan, journal, idempotency types stable.
- **Key strengths:** Clear state machine with validated transitions; serializable state and output; idempotency key format deterministic; comprehensive unit tests in crate.
- **Key risks:** `IdempotencyManager` is in-memory only (no persistent backend); `ExecutionContext` is minimal placeholder; no resume token type yet for suspend/resume.

## Target State

- **Production criteria:** Formal ExecutionState/ExecutionResult types for API; optional resume token for suspend/resume; idempotency persistent storage (see nebula-idempotency); schema snapshot for API compatibility.
- **Compatibility guarantees:** Patch/minor preserve public API and serialized form; breaking state or result shape only in major with MIGRATION.md.

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [MIGRATION.md](./MIGRATION.md)

## Design & research (in crate)

- [Execution View & Tracing](../../../crates/execution/docs/Execution-View-Tracing.md) — industry research for execution visibility and tracing (Inngest, Temporal, n8n, etc.).


