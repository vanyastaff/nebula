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
- `from_value::<WorkflowDefinition>()` fails — use `from_str(&to_string(&val))` (`ActionKey` has `#[serde(borrow)]`). `ExecutionState` is fine with `from_value`.
- `transition_status(Running)` fails if already Running — guard with `status != Running` before calling in resume.
- Credential refresh only fires when `credential_resolver` is also Some.
- Resume frontier is conservative (all predecessors terminal → eligible); activated-edge state not persisted.
- Resume budget/input not persisted — defaults used on resume (TODO).
- Idempotency: missing output on key hit → re-execute (partial write). Scope is per `execution_id`.

<!-- updated: 2026-04-07 — PR #230 review fixes -->
