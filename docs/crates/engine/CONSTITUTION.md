# nebula-engine Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula workflows are DAGs of nodes. Something must load the workflow definition, schedule node execution, pass data between nodes, handle results (success, wait, retry, fail), and persist execution state. That orchestrator is the engine.

**nebula-engine is the main workflow execution orchestrator.**

It answers: *Given a workflow ID and input, how does the platform schedule and run the DAG, pass data, handle action results, and expose execution state?*

```
API or trigger invokes engine.execute_workflow(workflow_id, input, options)
    ↓
Engine loads workflow, creates execution, builds ExecutionContext
    ↓
Scheduler plans node order; Executor runs nodes via runtime/sandbox
    ↓
State store persists execution state; EventBus emits ExecutionEvent
    ↓
Returns ExecutionHandle; execution continues until done, suspended, or failed
```

This is the engine contract: it owns workflow execution lifecycle and coordination; it does not implement individual actions or storage backends.

---

## User Stories

### Story 1 — API Starts a Workflow Run (P1)

An API receives a request to run workflow W with input JSON. It calls the engine; the engine creates an execution, schedules nodes, and returns a handle. The caller can poll or subscribe for completion.

**Acceptance**:
- `execute_workflow(workflow_id, input, options)` returns ExecutionHandle or error
- Execution is created with unique ExecutionId; state is persisted
- Events (Started, NodeCompleted, NodeFailed, Completed) emitted for observability

### Story 2 — Engine Passes Data Between Nodes (P1)

Node A produces output; node B consumes it via expression or input mapping. Engine passes data according to workflow topology and applies data limits (reject or spill to blob).

**Acceptance**:
- Execution context carries node outputs and execution metadata
- Data passing policy (max size, spill strategy) is enforced by runtime/engine contract
- Expression engine receives context built by engine/runtime

### Story 3 — Operator Observes Execution State (P2)

Operator needs to see running executions, completed results, and failures. Engine integrates with EventBus and optional metrics so that telemetry and API can expose state.

**Acceptance**:
- ExecutionEvent covers lifecycle and node outcomes
- State store allows query of execution status and result
- No business logic in engine that blocks observability

---

## Core Principles

### I. Engine Owns Execution Lifecycle, Not Action Implementation

**Engine schedules and coordinates; it does not implement Action trait or sandbox internals.**

**Rationale**: Actions and sandbox are in action/runtime/sandbox crates. Engine depends on them; it does not replace them.

**Rules**:
- Engine calls runtime (or executor) to run a node; runtime calls action
- No action-specific logic in engine; only result interpretation (ActionResult)

### II. Deterministic Scheduling When Possible

**For a given workflow DAG and input, node execution order is deterministic unless the workflow declares triggers or async waits.**

**Rationale**: Reproducibility and testing. Non-determinism only where explicitly required (e.g. wait for webhook).

**Rules**:
- Scheduler follows DAG topology; parallel branches are explicit
- Wait/suspend is first-class; resume is deterministic by design

### III. State Is Durable and Queryable

**Execution state is persisted so that restarts and queries can resume or inspect.**

**Rationale**: Long-running or suspended executions must survive process restart. Operators need to inspect state.

**Rules**:
- State store abstraction (engine may own or delegate to storage crate)
- ExecutionHandle or ID allows lookup of status and result

### IV. Events Are Fire-and-Forget for Engine Path

**Emitting events must not block execution. Lagging subscribers are dropped or buffered, not blocking.**

**Rationale**: Observability failures must not slow or fail the workflow. EventBus contract is best-effort for subscribers.

**Rules**:
- EventBus send is non-blocking
- Engine does not depend on delivery success for correctness

---

## Production Vision

### The engine in an n8n-class fleet

In production, the engine runs inside API or worker processes. It holds WorkflowEngine (scheduler, executor, state store, resource manager, event bus). Executions are created, scheduled, and run; node execution is delegated to runtime and sandbox. State is stored in configured backend; events flow to telemetry and metrics.

```
engine.rs   — WorkflowEngine: runtime (ActionRuntime), event_bus (EventBus), metrics (MetricsRegistry),
              action_keys (ActionId → registry key), plugin_registry (PluginRegistry), resolver (ParamResolver),
              expression_engine (ExpressionEngine), resource_manager (Option<Manager>)
              execute_workflow(workflow_id, definition, input, …) → ExecutionResult; frontier-based execution
resolver.rs — ParamResolver: resolves ParamValue (Literal, Expression, Template, Reference) via ExpressionEngine
result.rs   — ExecutionResult: execution_id, status (ExecutionStatus), node_outputs, duration
error.rs    — EngineError: ActionKeyNotFound, NodeNotFound, PlanningFailed, NodeFailed, Cancelled,
              ParameterResolution, ParameterValidation, EdgeEvaluationFailed, BudgetExceeded, Runtime, Execution, TaskPanicked
```

Engine uses: workflow (WorkflowDefinition, DependencyGraph, Connection, EdgeCondition, …), execution (ExecutionPlan, ExecutionState, ExecutionBudget, ExecutionStatus), action (ActionResult, NodeContext), runtime (ActionRuntime), expression (ExpressionEngine, EvaluationContext), plugin (PluginRegistry), telemetry (EventBus, ExecutionEvent, MetricsRegistry), resource (Manager optional). From the archives: `archive-node-execution.md` and engine archives describe scheduler, executor, state_store, resource_manager, event_bus; production vision aligns.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Full state store backend integration | High | Durable state for restart and query |
| Trigger lifecycle (start/stop) | Medium | Trigger types in action; engine orchestrates lifecycle |
| Backpressure and admission control | Medium | Integrate with system/memory pressure |
| Idempotency and deduplication | Low | Optional idempotency key per execution |

---

## Key Decisions

### D-001: Engine Depends on Runtime, Not Vice Versa

**Decision**: Engine calls runtime to execute a node; runtime does not call engine for scheduling.

**Rationale**: Clear dependency direction; runtime is reusable by other entry points (e.g. tests, CLI).

**Rejected**: Runtime holding reference to engine — would create cycle.

### D-002: ExecutionContext Built by Engine

**Decision**: Engine builds execution context (execution_id, workflow_id, workflow, options); runtime receives it.

**Rationale**: Engine owns execution lifecycle; context is the handoff to runtime and actions.

**Rejected**: Runtime building context from scratch — would duplicate engine state.

### D-003: EventBus for Observability, Not Control Flow

**Decision**: Events are for logging, metrics, and API subscription; execution flow does not depend on delivery.

**Rationale**: Prevents observability failures from failing workflows.

**Rejected**: Synchronous delivery requirement — would couple engine to subscribers.

---

## Open Proposals

### P-001: Trigger Lifecycle in Engine

**Problem**: Trigger-based workflows need register/unregister and start/stop.

**Proposal**: Engine (or dedicated service) owns trigger lifecycle; action defines TriggerContext and trigger types.

**Impact**: New engine API and possibly new crate for trigger registry.

### P-002: Backpressure and Admission

**Problem**: Under load, engine should reject or queue new executions based on system/memory pressure.

**Proposal**: Integrate with nebula-system pressure events and optional admission controller.

**Impact**: Engine subscribes to pressure; scheduler or API gate applies policy.

---

## Non-Negotiables

1. **Engine owns execution lifecycle** — create, schedule, persist state, emit events.
2. **No action implementation in engine** — engine uses runtime/sandbox to run actions.
3. **Events do not block execution** — fire-and-forget; best-effort delivery.
4. **State is durable** — execution state survives process restart where configured.
5. **Deterministic scheduling** — order defined by DAG and explicit wait/trigger.
6. **Breaking execution or context contract = major + MIGRATION.md** — API and workers depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to execute_workflow or context contract.
- **MINOR**: Additive (new options, new event fields). No removal of existing behavior.
- **MAJOR**: Breaking changes to execution lifecycle or context. Requires MIGRATION.md.
