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
- **`ExecutionTerminationReason` vs `ExecutionStatus`.** Status answers *what* terminal state (`Completed` / `Failed` / `Cancelled` / `TimedOut`). `ExecutionTerminationReason` answers *why* — `NaturalCompletion` / `ExplicitStop { by_node, note }` / `ExplicitFail { by_node, code: ExecutionTerminationCode, message }` / `Cancelled` / `SystemError`. Attached to `ExecutionResult` as `Option<ExecutionTerminationReason>` (`#[serde(default)]` for backward compat — legacy results deserialize as `None`). `code` is the public newtype `ExecutionTerminationCode` (`#[serde(transparent)]` over `Arc<str>`) — this pins the wire format so the internal representation can swap to a structured `ErrorCode` in Phase 10 of the action-v2 roadmap without changing the public API or serialized shape. Audit log and UI use this to distinguish a workflow that `StopAction`/`FailAction` ended deliberately from a crash — do not collapse the two. Added 2026-04-13 as Phase 0 of `ControlAction` work.
- **Phase 0 `Terminate` wiring is partial.** The engine only gates downstream edges locally via `evaluate_edge` when a node returns `ActionResult::Terminate`; `run_frontier`, `determine_final_status`, `check_and_apply_idempotency`, and `resume_execution` do NOT yet consume `TerminationReason` or set `ExecutionResult::termination_reason`. Full parallel-branch cancellation and audit-log propagation via `ExecutionTerminationReason::ExplicitStop` / `::ExplicitFail` is scheduler work tracked as Phase 3 in `docs/plans/2026-04-13-control-action-plan.md`. Until that lands, `StopAction`/`FailAction` end their own branch's downstream traffic but leave sibling branches running, and the `termination_reason` field stays `None` on `ExecutionResult` even when `Terminate` was returned — treat it as best-effort signalling, not enforcement.

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-storage, nebula-api.

<!-- reviewed: 2026-04-14 — #247 added ExecutionTerminationReason + ExecutionTerminationCode newtype and ExecutionResult::termination_reason field; Phase 0 wiring is partial (local edge gate only, full scheduler propagation deferred) -->
