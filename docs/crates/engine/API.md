# API

## Public Surface

- **Stable:** `WorkflowEngine`, `execute_workflow`, `ExecutionResult`, `EngineError` and variants. Execution lifecycle and result shape are contract for API/workers.
- **Experimental:** Optional trigger lifecycle, admission control APIs when added.
- **Hidden/internal:** ParamResolver, scheduler internals, event emission details.

## Usage Patterns

- **API/worker:** Build WorkflowEngine with runtime, event_bus, metrics, plugin_registry, expression_engine, optional resource_manager. On request, call `execute_workflow(workflow_id, definition, input, options)`; use ExecutionResult for response or polling; subscribe to EventBus for real-time events.
- **Testing:** Construct engine with mock runtime and event bus; assert ExecutionResult and event ordering.

## Minimal Example

```rust
let engine = WorkflowEngine::builder()
    .runtime(runtime)
    .event_bus(event_bus)
    .plugin_registry(registry)
    .expression_engine(expr_engine)
    .build()?;
let result = engine.execute_workflow(workflow_id, definition, input, options).await?;
// result.execution_id, result.status, result.node_outputs
```

## Error Semantics

- **Retryable:** EngineError::NodeFailed with retryable action error; engine may retry per resilience policy. Runtime/Execution errors that are classified retryable.
- **Fatal:** EngineError::PlanningFailed, NodeNotFound, ActionKeyNotFound, Cancelled, ParameterValidation, etc. Caller should not retry same input without fix.
- **Validation:** ParameterResolution, ParameterValidation, EdgeEvaluationFailed — fix input or workflow.

## Compatibility Rules

- **Major bump:** Breaking change to execute_workflow signature, ExecutionResult, or execution/context contract. MIGRATION.md required.
- **Minor:** Additive (new options, new event fields, new error variants that are additive). No removal.
- **Deprecation:** At least one minor version with notice before removal.
