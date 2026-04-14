# nebula-execution
Execution state machine types — persistent state, journals, idempotency, plans. NOT the orchestrator.

## Invariants
- This crate is data types only. The orchestration logic is in nebula-engine. The "execution" name can mislead.
- `ExecutionStatus` has 8 states with enforced transitions via the `transition` module. Invalid transitions return errors.

## Key Decisions
- `ExecutionPlan` is a pre-computed parallel schedule (levels) built from the workflow graph — computed by nebula-engine, consumed here as a data type.
- `IdempotencyKey` is a deterministic `{execution}:{node}:{attempt}` token. Deduplication is owned by `nebula_storage::ExecutionRepo::{check_idempotency, mark_idempotent}` — there is no in-memory `IdempotencyManager` (deleted in batch 5C, issue #303) because a process-local `HashSet` cache duplicated repo state and disappeared on restart.
- `JournalEntry` provides an immutable audit log of execution events — append-only.
- **`ReplayPlan` partition contract (issues #253, #254):** `partition_nodes(all_nodes, successors)` returns `(pinned, rerun)` where `rerun = {replay_from} ∪ forward-reachable(replay_from)` and `pinned = all_nodes \ rerun`. Pinned nodes include ancestors, unrelated siblings, and disconnected branches — every non-rerun node MUST have a stored output in `pinned_outputs`. The engine's `replay_execution` iterates the pinned set and errors with `PlanningFailed` on any missing entry; this is deliberate, the previous filter-on-key approach hid silent data corruption. `pinned_outputs` serializes normally (no `#[serde(skip)]`) so plans round-trip through storage without losing stored outputs.

## Traps
- **Name confusion**: `nebula-execution` ≠ execution engine. If you want the scheduler, look at nebula-engine.
- **`ExecutionBudget.max_concurrent_nodes` must be ≥ 1.** Zero maps to `Semaphore::new(0)` in the engine and deadlocks the scheduler (no permits). Serde rejects `0`; callers constructing structs manually or via `..default()` must use `validate_for_execution()` or the builder (`with_max_concurrent_nodes` asserts).
- `ExecutionContext` here is lightweight (execution_id + budget) — different from `ActionContext` in nebula-action which has DI capabilities.
- `NodeAttempt` tracks individual retry attempts for a node. `NodeExecutionState` tracks overall node status. Both are needed.
- **`ExecutionTerminationReason` vs `ExecutionStatus`.** Status answers *what* terminal state (`Completed` / `Failed` / `Cancelled` / `TimedOut`). `ExecutionTerminationReason` answers *why* — `NaturalCompletion` / `ExplicitStop { by_node, note }` / `ExplicitFail { by_node, code: ExecutionTerminationCode, message }` / `Cancelled` / `SystemError`. Attached to `ExecutionResult` as `Option<ExecutionTerminationReason>` (`#[serde(default)]` for backward compat — legacy results deserialize as `None`). `code` is the public newtype `ExecutionTerminationCode` (`#[serde(transparent)]` over `Arc<str>`) — this pins the wire format so the internal representation can swap to a structured `ErrorCode` in Phase 10 of the action-v2 roadmap without changing the public API or serialized shape. Audit log and UI use this to distinguish a workflow that `StopAction`/`FailAction` ended deliberately from a crash — do not collapse the two. Added 2026-04-13 as Phase 0 of `ControlAction` work.
- **Phase 0 `Terminate` wiring is partial.** The engine only gates downstream edges locally via `evaluate_edge` when a node returns `ActionResult::Terminate`; `run_frontier`, `determine_final_status`, `check_and_apply_idempotency`, and `resume_execution` do NOT yet consume `TerminationReason` or set `ExecutionResult::termination_reason`. Full parallel-branch cancellation and audit-log propagation via `ExecutionTerminationReason::ExplicitStop` / `::ExplicitFail` is scheduler work tracked as Phase 3 in `docs/plans/2026-04-13-control-action-plan.md`. Until that lands, `StopAction`/`FailAction` end their own branch's downstream traffic but leave sibling branches running, and the `termination_reason` field stays `None` on `ExecutionResult` even when `Terminate` was returned — treat it as best-effort signalling, not enforcement.

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-storage, nebula-api.

<!-- reviewed: 2026-04-14 — #247 added ExecutionTerminationReason + ExecutionTerminationCode newtype and ExecutionResult::termination_reason field; Phase 0 wiring is partial (local edge gate only, full scheduler propagation deferred) -->

<!-- reviewed: 2026-04-14 — PR #388 review-fixup: doc-comment on ExecutionBudget.max_concurrent_nodes (intra-doc link → backticks). No invariants changed. -->

<!-- reviewed: 2026-04-14 — fixed rustdoc intra-doc link in ExecutionBudget docs (`tokio::sync::Semaphore` referenced as code, not unresolved intra-doc link) -->
