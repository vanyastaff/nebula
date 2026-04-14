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
- **`resolved_edges` is `HashMap<NodeId, usize>` (edge-count, not source-node set).** Using `HashSet<NodeId>` caused multi-edge stalls: two edges from A→B would dedup to one entry, so resolved never reached required=2. The count-based type correctly increments once per edge processed.
- `activated_edges` remains `HashMap<NodeId, HashSet<NodeId>>` (source-node set) — used by `resolve_node_input_with_support` to filter which predecessor outputs to include; dedup-by-source is correct there.
- `determine_final_status` guards `Completed` behind `exec_state.all_nodes_terminal()`. If frontier drains but non-terminal nodes remain (engine bug), returns `Failed` with a warning rather than a false `Completed`.

## resume_execution()
- Needs `execution_repo` + `workflow_repo` or returns `PlanningFailed`.
- Rejects terminal executions. Running nodes reset to Pending.

## Traps
- **`ExecutionBudget.max_concurrent_nodes == 0` is rejected at `execute_workflow` / `replay_execution` entry** (`PlanningFailed`). The frontier uses `Semaphore::new(max_concurrent_nodes)`; zero permits deadlock forever. (Serde and CLI also reject 0.)
- `from_value::<WorkflowDefinition>()` fails — use `from_str(&to_string(&val))`.
- `transition_status(Running)` fails if already Running — guard before calling.
- Credential refresh only fires when `credential_resolver` is also Some.
- Resume budget/input not persisted — defaults on resume (TODO).
- **`evaluate_edge` gates four `ActionResult` variants, not three.** Skip, Drop, and Terminate all return `false` unconditionally (no downstream edges activate); only `Branch`/`Route`/`MultiOutput` do selector-based filtering. `Drop` and `Terminate` were added 2026-04-13 as Phase 0 of the `ControlAction` work — previously a control-flow node returning either variant would have fallen through to `EdgeCondition::Always` and silently fired downstream. When adding new `ActionResult` variants, decide up front whether they should gate or fall through and update `evaluate_edge` in the same PR. Full parallel-branch cancellation for `Terminate` (cancelling sibling branches and recording `ExecutionTerminationReason::ExplicitStop`/`::ExplicitFail` in the audit log) is engine/scheduler work tracked separately from the edge gate — the gate just prevents the local downstream edges from firing between the terminate signal and whatever engine-level teardown eventually lands.

<!-- reviewed: 2026-04-14 — #247 added Drop/Terminate gate to evaluate_edge with TODO(engine) for Phase 3 scheduler wiring -->

<!-- reviewed: 2026-04-14 — multi-edge fix: resolved_edges changed to usize count; determine_final_status guards Completed on all_nodes_terminal() -->
