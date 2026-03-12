# nebula-engine

Workflow execution orchestrator for Nebula. Loads workflow definitions, builds execution plan, runs nodes frontier-by-frontier via runtime, resolves parameters (expressions, references), and emits telemetry.

## Scope

- **In scope:**
  - **engine** — `WorkflowEngine`: holds ActionRuntime, EventBus, MetricsRegistry, action_keys (ActionId → registry key), PluginRegistry, ParamResolver, ExpressionEngine, optional ResourceManager; `execute_workflow(workflow_id, definition, input, …)` → `ExecutionResult`; frontier-based scheduling (nodes run as soon as predecessors are resolved).
  - **resolver** (internal) — `ParamResolver`: resolves node `ParamValue` (Literal, Expression, Template, Reference) using ExpressionEngine and predecessor outputs.
  - **result** — `ExecutionResult`: execution_id, status (ExecutionStatus from nebula-execution), node_outputs, duration.
  - **error** — `EngineError`: ActionKeyNotFound, NodeNotFound, PlanningFailed, NodeFailed, Cancelled, ParameterResolution, ParameterValidation, EdgeEvaluationFailed, BudgetExceeded, Runtime, Execution, TaskPanicked.
- **Out of scope:** Action implementations; storage backends (engine uses execution/state types); sandbox internals (runtime/sandbox own those).

## Dependencies

- **nebula-***: core, action, expression, plugin, parameter, workflow, execution, resource, runtime, telemetry.
- **Vendor:** async-trait, tokio, tokio-util, dashmap, thiserror, tracing, serde_json.

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md) — problem, current/target architecture
- [API.md](./API.md) — public surface, execute_workflow, ExecutionResult, compatibility
- [ROADMAP.md](./ROADMAP.md) — phases, risks, exit criteria
- [MIGRATION.md](./MIGRATION.md) — versioning, breaking changes, rollout/rollback


