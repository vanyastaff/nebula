# nebula-execution

Execution state machine, journals, idempotency, and planning for the Nebula workflow engine. This crate models execution-time concepts only — it does not contain the engine orchestrator.

## Scope

- **In scope:**
  - `ExecutionStatus` — execution-level state machine (8 states: Created, Running, Paused, Cancelling, Completed, Failed, Cancelled, TimedOut)
  - `ExecutionState` and `NodeExecutionState` — persistent state tracking with validated transitions
  - `ExecutionPlan` — pre-computed parallel execution schedule from workflow definition
  - `ExecutionContext` — lightweight runtime context (execution_id, budget)
  - `JournalEntry` — audit log of execution events
  - `NodeOutput` / `ExecutionOutput` — node output data (inline JSON or blob ref) with metadata
  - `NodeAttempt` — individual attempt tracking with idempotency key
  - `IdempotencyKey` and `IdempotencyManager` — exactly-once guarantees (in-memory)
  - Transition validation: `validate_execution_transition`, `validate_node_transition` (via `nebula_workflow::NodeState`)

- **Out of scope:**
  - Workflow scheduling and DAG orchestration (engine)
  - Persistence of execution state (engine/storage)
  - Action execution (runtime)

## Current State

- **Maturity:** Implemented and tested; state machine, plan, journal, idempotency types stable.
- **Key strengths:** Clear state machine with validated transitions; serializable state and output; idempotency key format deterministic; comprehensive unit tests in crate.
- **Key risks:** `IdempotencyManager` is in-memory only (no persistent backend); `ExecutionContext` is minimal placeholder; no resume token type yet for suspend/resume.

## Target State

- **Production criteria:** Formal ExecutionState/ExecutionResult types for API; optional resume token for suspend/resume; idempotency persistent storage (see nebula-idempotency); schema snapshot for API compatibility.
- **Compatibility guarantees:** Patch/minor preserve public API and serialized form; breaking state or result shape only in major with MIGRATION.md.

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
