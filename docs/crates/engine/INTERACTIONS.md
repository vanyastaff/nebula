# Interactions

## Ecosystem Map

**nebula-engine** is the workflow execution orchestrator. It depends on many nebula-* crates; nothing in the workspace depends on engine (API, worker, or desktop app would).

### Upstream (engine depends on)

- **nebula-core** — ActionId, ExecutionId, NodeId, WorkflowId.
- **nebula-action** — ActionResult, NodeContext; action contract.
- **nebula-expression** — ExpressionEngine, EvaluationContext; parameter and edge condition evaluation.
- **nebula-plugin** — PluginRegistry, PluginKey, PluginMetadata; action lookup.
- **nebula-parameter** — parameter schema (validation).
- **nebula-workflow** — WorkflowDefinition, DependencyGraph, Connection, EdgeCondition, ResultMatcher, ErrorMatcher, NodeState, ParamValue.
- **nebula-execution** — ExecutionPlan, ExecutionState, ExecutionBudget, ExecutionStatus; execution lifecycle types.
- **nebula-runtime** — ActionRuntime; runs actions.
- **nebula-resource** — Manager (optional); resources for actions.
- **nebula-telemetry** — EventBus, ExecutionEvent, MetricsRegistry; observability.
- **Vendor:** tokio, dashmap, thiserror, tracing, serde_json.

### Downstream (consume engine)

- No workspace crate depends on nebula-engine. **Typical consumers:** API server, worker binary, desktop app — they call `WorkflowEngine::execute_workflow` and use `ExecutionResult`.

### Planned

- **api** — HTTP layer that invokes engine for run workflow.
- **worker** — background runner that pulls jobs and executes via engine.

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|--------------------|-----------|----------|------------|------------------|-------|
| engine → workflow | in | WorkflowDefinition, DependencyGraph, validate, topological/levels | sync | WorkflowError | load and plan |
| engine → execution | in | ExecutionPlan, ExecutionState, ExecutionBudget, ExecutionStatus | async | ExecutionError | state and plan |
| engine → runtime | in | ActionRuntime::execute_node | async | RuntimeError → EngineError | run actions |
| engine → action | in | ActionResult, NodeContext | sync | ActionError mapped | interpret result |
| engine → expression | in | ExpressionEngine, EvaluationContext | sync | expression errors → EngineError | param/edge eval |
| engine → plugin | in | PluginRegistry, action key lookup | sync | N/A | resolve action_id → handler |
| engine → telemetry | out | EventBus.emit(ExecutionEvent), MetricsRegistry | best-effort | non-blocking | observability |
| engine → resource | in | Manager (optional) | async | N/A | provide resources to actions |

## Runtime Sequence

1. Caller invokes `engine.execute_workflow(workflow_id, definition, input, options)`.
2. Engine builds DependencyGraph, ExecutionPlan; creates execution state (ExecutionState/ExecutionBudget).
3. Frontier-based loop: for each node whose predecessors are done, resolve params (ParamResolver + ExpressionEngine), optionally validate (parameter schema), call runtime.execute_node(…); handle ActionResult (Success, Branch, Wait, Retry, …) and ActionError (Retryable, Fatal).
4. Emit ExecutionEvent (Started, NodeCompleted, NodeFailed, Completed); update metrics.
5. Return ExecutionResult (execution_id, status, node_outputs, duration).

## Cross-Crate Ownership

- **engine** owns: execution lifecycle orchestration, scheduling, param resolution, event emission, mapping action_id → registry key.
- **runtime** owns: invoking action implementation, sandbox boundary.
- **execution** owns: execution state types, plan, status enum.
- **workflow** owns: definition and DAG structure; engine only consumes.

## Versioning and Compatibility

- Execution lifecycle and ExecutionResult shape are contracts for API/workers. Breaking = major + MIGRATION.md.
