# nebula-engine
Workflow execution orchestrator — frontier-based DAG scheduler, node dispatch.

## Invariants
- Delegates action execution to `ActionRuntime` — never runs actions directly.
- `EngineCredentialAccessor`: empty `allowed_keys` = allow all. Non-empty = strict allowlist.
- `credential_resolver: None` → noop. `resource_manager: None` → noop.
- `with_credential_resolver()` type-erases the resolver fn.

## Key Decisions
- Frontier-based: nodes spawn when all incoming edges resolve.
- Budget in `check_budget()`: `max_duration`, `max_output_bytes`, `max_total_retries`.
- Error strategy via `handle_node_failure()`: FailFast cancels, ContinueOnError skips dependents, IgnoreErrors = null success.
- Disabled nodes: `mark_node_skipped()` + `process_outgoing_edges(None, None)`. NOT `propagate_skip`.
- `run_frontier` takes explicit `seed_nodes`, `initial_activated`, `initial_resolved` — fresh start passes entry nodes + empty maps.

## resume_execution()
- Needs both `execution_repo` + `workflow_repo` or returns `PlanningFailed`.
- Rejects terminal executions. Running nodes reset to Pending.
- Frontier = non-terminal nodes with all predecessors terminal.

## Traps
- `pub(crate) resolver` — internal.
- `ExecutionResult` (engine) ≠ `ExecutionState` (persistent).
- `nebula-credential` direct dep (for `CredentialSnapshot`).
- `from_value::<WorkflowDefinition>()` fails — `ActionKey` has `#[serde(borrow)]`. Use `from_str(&to_string(&val))`.

<!-- updated: 2026-04-07 — EF1: workflow_repo, resume_execution(), run_frontier refactored -->
