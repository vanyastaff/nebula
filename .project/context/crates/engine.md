# nebula-engine
Workflow execution orchestrator — frontier-based DAG scheduler.

## Invariants
- Delegates action execution to `ActionRuntime` — never runs actions directly.
- `credential_resolver: None` → noop. `resource_manager: None` → noop.

## Key Decisions
- `ExecutionResult` carries `node_outputs` + `node_errors`.
- `with_event_sender()` — optional mpsc for real-time monitoring (TUI).
- Frontier-based: nodes spawn when all incoming edges resolve.
- ErrorStrategy: FailFast cancels, ContinueOnError skips dependents, IgnoreErrors = null success.
- Disabled nodes: `mark_node_skipped()` + `process_outgoing_edges(None, None)`.

## resume_execution()
- Needs `execution_repo` + `workflow_repo` or returns `PlanningFailed`.
- Rejects terminal executions. Running nodes reset to Pending.

## Traps
- `ExecutionBudget.max_concurrent_nodes == 0` rejected at entry — zero permits deadlock the `Semaphore`.
- `from_value::<WorkflowDefinition>()` fails — use `from_str(&to_string(&val))`.
- `transition_status(Running)` fails if already Running — guard before.
- Credential refresh only fires when `credential_resolver` is also Some.
- **Cred refresh failure is typed, not WARN-and-continue** (#306, B5D). Hook `Err` → `EngineError::Action(CredentialRefreshFailed)` → `handle_node_failure` → workflow `ErrorStrategy`. Action body never invoked. Awaited under `tokio::select!` vs cancel. Default: retryable. No per-action overrides; no retry/dead-letter here.
- Resume budget/input not persisted — defaults on resume (TODO).
- **Failure finalization ordering is strategy-sensitive.** `handle_node_failure()` can rewrite a node from `Failed` to `Completed` (`IgnoreErrors`) and/or inject synthetic outputs for `OnError` routing. Do not checkpoint or emit `NodeFailed` before this call; persist/emit only after confirming the node still ends in `Failed`.
- **Budget timeout/cancel teardown must abort JoinSet tasks before drain.** A cancelled token alone does not guarantee in-flight action tasks will cooperatively exit; call `join_set.abort_all()` before draining so max-duration/cancel paths cannot hang indefinitely.
- **`evaluate_edge` gates `Skip`/`Drop`/`Terminate` unconditionally** (return `false`); only `Branch`/`Route`/`MultiOutput` do selector filtering. `Drop`/`Terminate` added 2026-04-13 (ControlAction Phase 0). New `ActionResult` variants: decide gate-vs-fall-through and update `evaluate_edge` in the same PR. Full `Terminate` sibling-branch cancellation tracked separately.

<!-- reviewed: 2026-04-14 — #247 added Drop/Terminate gate; PR #394 added failure-ordering + JoinSet abort_all traps -->
<!-- reviewed: 2026-04-14 -->
