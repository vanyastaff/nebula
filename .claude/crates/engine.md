# nebula-engine
Workflow execution orchestrator — DAG scheduler, level-by-level execution, node dispatch.

## Invariants
- Engine sits between the user-facing API and nebula-runtime. It does not execute actions directly — it delegates to `ActionRuntime`.
- Currently blocked: credential/resource injection not wired. Engine can schedule but cannot fully inject DI into actions yet.

## Key Decisions
- Execution is level-by-level: builds `ExecutionPlan` from `DependencyGraph` (from nebula-workflow), then executes each level in parallel with bounded concurrency.
- Node inputs are resolved from predecessor outputs (output wiring). `resolver` module handles this.
- `WorkflowEngine` re-exports `PluginRegistry` — callers register plugins into the engine, not separately.

## Traps
- Engine is **blocked** on nebula-resource stabilization. Don't invest heavily in engine internals until resource/credential DI is stable.
- `pub(crate) resolver` — the input resolution logic is internal. Don't expose it without design review.
- `result::ExecutionResult` is distinct from `execution::ExecutionState`. The former is the engine's final return type; the latter is persistent state in nebula-execution.

## Relations
- Depends on nebula-workflow, nebula-execution, nebula-action, nebula-plugin, nebula-runtime. Used by nebula-api.

<\!-- reviewed: 2026-03-25 -->
