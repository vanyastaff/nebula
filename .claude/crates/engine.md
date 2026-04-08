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

## EF2–4 decisions
- Idempotency key: `"{execution_id}:{node_id}:1"`. Only active with `execution_repo`. `check_and_apply_idempotency` short-circuits; `record_idempotency` fires post-success.
- Version pinning: `NodeTask.interface_version` → `execute_action_versioned`. `None` = latest.
- Credential refresh hook: called in `NodeTask::run` before dispatch; errors logged, not fatal.

## Traps
- `from_value::<WorkflowDefinition>()` fails — use `from_str(&to_string(&val))` (`ActionKey` has `#[serde(borrow)]`).
- Idempotency scope is per `execution_id` — not portable across executions.

<!-- updated: 2026-04-07 — EF2: idempotency, EF3: version pinning, EF4: credential refresh -->
