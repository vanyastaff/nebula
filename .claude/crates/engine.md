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

<!-- reviewed: 2026-04-09 — Phase 7.5: test handlers migrated from InternalHandler to StatelessAction, no architectural change -->
<!-- reviewed: 2026-04-10 — mechanical import path update: nebula_action::execution::StatelessAction → nebula_action::stateless::StatelessAction (action crate module layout cleanup) -->
