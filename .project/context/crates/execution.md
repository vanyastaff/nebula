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

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-storage, nebula-api.

<!-- reviewed: 2026-03-30 — derive Classify migration -->

<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-06 �� added Cancelling→Completed and Cancelling→TimedOut transitions -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
