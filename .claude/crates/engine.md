# nebula-engine
Workflow execution orchestrator — DAG scheduler, level-by-level execution, node dispatch.

## Invariants
- Sits between API and nebula-runtime. Does not execute actions directly — delegates to `ActionRuntime`.
- Resource manager wired via `with_resource_manager()`. Credential injection still not wired.

## Key Decisions
- Level-by-level execution with bounded concurrency from `ExecutionPlan`/`DependencyGraph`.
- `resource` module bridges engine context to `nebula_resource::Manager` for per-node acquisition.
- Execution repo removed — persistent checkpointing deferred until storage stabilizes.
- Error handling simplified: `process_outgoing_edges` replaces former `handle_node_failure` + `ErrorStrategy`.
- No EventBus — metrics only. Events will be redesigned when engine stabilizes.

## Traps
- Still needs credential DI (`CredentialResolver` into `ActionContext`) for end-to-end execution.
- `pub(crate) resolver` — internal input resolution. Don't expose without design review.
- `result::ExecutionResult` ≠ `execution::ExecutionState` (engine return type vs persistent state).

## Relations
- Depends on nebula-workflow, nebula-execution, nebula-action, nebula-plugin, nebula-runtime, nebula-resource. Used by nebula-api.

<!-- reviewed: 2026-04-07 — resource manager wired, execution repo removed, error handling simplified -->
