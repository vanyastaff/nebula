# nebula-execution
Execution state machine types — persistent state, journals, idempotency, plans. NOT the orchestrator.

## Invariants
- This crate is data types only. The orchestration logic is in nebula-engine. The "execution" name can mislead.
- `ExecutionStatus` has 8 states with enforced transitions via the `transition` module. Invalid transitions return errors.

## Key Decisions
- `ExecutionPlan` is a pre-computed parallel schedule (levels) built from the workflow graph — computed by nebula-engine, consumed here as a data type.
- `IdempotencyManager` enforces exactly-once execution via `IdempotencyKey`. Use it before re-running nodes.
- `JournalEntry` provides an immutable audit log of execution events — append-only.

## Traps
- **Name confusion**: `nebula-execution` ≠ execution engine. If you want the scheduler, look at nebula-engine.
- `ExecutionContext` here is lightweight (execution_id + budget) — different from `ActionContext` in nebula-action which has DI capabilities.
- `NodeAttempt` tracks individual retry attempts for a node. `NodeExecutionState` tracks overall node status. Both are needed.
- **`ExecutionTerminationReason` vs `ExecutionStatus`.** Status answers *what* terminal state (`Completed` / `Failed` / `Cancelled` / `TimedOut`). `ExecutionTerminationReason` answers *why* — `NaturalCompletion` / `ExplicitStop { by_node, note }` / `ExplicitFail { by_node, code: Arc<str>, message }` / `Cancelled` / `SystemError`. Attached to `ExecutionResult` as `Option<ExecutionTerminationReason>` (`#[serde(default)]` for backward compat — legacy results deserialize as `None`). `code` in `ExplicitFail` is a placeholder `Arc<str>` until `ErrorCode` lands. Audit log and UI use this to distinguish a workflow that `StopAction`/`FailAction` ended on purpose from a crash — do not collapse the two. Added 2026-04-13 as Phase 0 of `ControlAction` work.

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-storage, nebula-api.
