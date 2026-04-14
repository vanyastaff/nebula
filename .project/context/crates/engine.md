# nebula-engine
Workflow execution orchestrator — frontier-based DAG scheduler.

## Invariants
- Delegates action execution to `ActionRuntime` — never runs actions directly.
- `credential_resolver: None` → noop. `resource_manager: None` → noop.

## Key Decisions
- `ExecutionResult` has `node_outputs` + `node_errors` per node.
- `ExecutionEvent` + `with_event_sender()` — optional mpsc for real-time monitoring (TUI).
- Frontier-based: nodes spawn when all incoming edges resolve.
- Error strategy: FailFast cancels, ContinueOnError skips dependents, IgnoreErrors = null success.
- Disabled nodes: `mark_node_skipped()` + `process_outgoing_edges(None, None)`.

## resume_execution()
- Needs `execution_repo` + `workflow_repo` or returns `PlanningFailed`.
- Rejects terminal executions. Running nodes reset to Pending.

## Traps
- `from_value::<WorkflowDefinition>()` fails — use `from_str(&to_string(&val))`.
- `transition_status(Running)` fails if already Running — guard before calling.
- Credential refresh only fires when `credential_resolver` is also Some.
- Resume budget/input not persisted — defaults on resume (TODO).
- **`evaluate_edge` gates four `ActionResult` variants, not three.** Skip, Drop, and Terminate all return `false` unconditionally (no downstream edges activate); only `Branch`/`Route`/`MultiOutput` do selector-based filtering. `Drop` and `Terminate` were added 2026-04-13 as Phase 0 of the `ControlAction` work — previously a control-flow node returning either variant would have fallen through to `EdgeCondition::Always` and silently fired downstream. When adding new `ActionResult` variants, decide up front whether they should gate or fall through and update `evaluate_edge` in the same PR. Full parallel-branch cancellation for `Terminate` (cancelling sibling branches and recording `ExecutionTerminationReason::ExplicitStop`/`::ExplicitFail` in the audit log) is engine/scheduler work tracked separately from the edge gate — the gate just prevents the local downstream edges from firing between the terminate signal and whatever engine-level teardown eventually lands.

<!-- reviewed: 2026-04-14 — #247 added Drop/Terminate gate to evaluate_edge with TODO(engine) for Phase 3 scheduler wiring -->
