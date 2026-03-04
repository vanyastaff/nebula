# Interactions

## Crate boundaries

| Crate | Responsibility |
|-------|-----------------|
| **nebula-execution** | State and model only: execution state machine, plan, journal, idempotency; no orchestration, no action execution. |
| **nebula-action** | Action contract: traits (StatelessAction, etc.), ActionContext/TriggerContext, ActionResult, ActionError; no scheduling. |
| **nebula-runtime** (this crate) | Action execution: registry lookup, context build, run via sandbox, data limits, telemetry; one node at a time. |
| **nebula-engine** | DAG orchestration: builds state/plan, applies transitions, persists; calls runtime to run nodes. |

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-engine` | Downstream | Uses ActionRuntime for node execution; creates NodeTask with runtime |
| `nebula-action` | Upstream | ActionResult, ActionError, Context/ActionContext; execution traits (StatelessAction, etc.); trigger types (webhook, schedule) |
| `nebula-plugin` | Upstream | InternalHandler trait (execute with context); handlers registered by key; runtime uses for lookup |
| (this crate) | — | SandboxRunner, TaskQueue traits and InProcessSandbox, MemoryQueue in nebula-runtime |
| `nebula-telemetry` | Upstream | EventBus, ExecutionEvent, MetricsRegistry |
| `nebula-core` | Upstream | Id types (indirect via action) |
| `nebula-execution` | Sibling | ExecutionPlan, ExecutionState; engine uses; runtime does not |

### Planned / Optional

| Crate | Relationship | Description |
|-------|-------------|-------------|
| (this crate) | — | InProcessSandbox, MemoryQueue built-in |

## Context contract (current vs target)

- **Current:** `execute_action(action_key, input, NodeContext)`. Runtime and plugin use `NodeContext` (deprecated in nebula-action). InternalHandler takes NodeContext.
- **Target:** Align with nebula-action: build `ActionContext` (or TriggerContext for triggers), pass as `&impl Context`; handlers may adopt StatelessAction/StatefulAction. Migration: see PROPOSALS (ActionContext/TriggerContext in runtime).

## Downstream Consumers

### nebula-engine

- **Expectations:** `Arc<ActionRuntime>`; `execute_action(action_key, input, context)` returns `Result<ActionResult, RuntimeError>`. Context type is currently NodeContext; may migrate to ActionContext.
- **Contract:** Async; engine maps RuntimeError to EngineError::Runtime
- **Usage:** NodeTask.run() calls runtime.execute_action; engine resolves action_key from node definition

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `nebula-action` | Context type (currently NodeContext), ActionResult, ActionError | execute signature | — |
| `nebula-plugin` | InternalHandler | metadata(), execute() | — |
| (this crate) | SandboxRunner | execute through sandbox | — |
| `nebula-telemetry` | EventBus, MetricsRegistry | emit, counter, histogram | — |
| `nebula-core` | (indirect) | — | — |
| `dashmap` | ActionRegistry storage | concurrent map | — |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|----------------------|-----------|----------|------------|------------------|-------|
| runtime -> engine | out | execute_action | async | Result<RuntimeError> | Engine catches and maps |
| runtime -> action | in | handler.execute() | async | ActionError | Wrapped in RuntimeError |
| runtime -> plugin | in | InternalHandler | async | — | Registry stores Arc<dyn InternalHandler> |
| runtime -> (sandbox) | in | SandboxRunner | async | — | In-crate sandbox module |
| runtime -> telemetry | out | emit, metrics | sync | best-effort | Fire-and-forget |

## Runtime Sequence

1. Engine builds NodeTask with runtime, action_key, input, context.
2. NodeTask.run() acquires semaphore, checks cancellation.
3. NodeTask calls `runtime.execute_action(action_key, input, context)`.
4. ActionRuntime: registry.get() → emit NodeStarted → handler.execute() → check data limit → emit NodeCompleted/NodeFailed → record metrics.
5. NodeTask extracts primary output, inserts into outputs map.
6. Engine processes result, evaluates edges, continues frontier.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| Action execution orchestration | `nebula-runtime` |
| Action handler implementations | `nebula-plugin`, plugins |
| Sandbox implementation | This crate (SandboxRunner, InProcessSandbox) |
| Workflow scheduling | `nebula-engine` |
| Event schema | `nebula-telemetry` |
| Data policy config | `nebula-runtime` (DataPassingPolicy) |

## Failure Propagation

- **ActionError:** Propagates as RuntimeError::ActionError; engine maps to EngineError::Runtime.
- **ActionNotFound:** Registry lookup fails; returned immediately.
- **DataLimitExceeded:** After execution; emit NodeFailed, return error.
- **Retryable:** RuntimeError::is_retryable() delegates to ActionError::is_retryable().

## Versioning and Compatibility

- **Compatibility promise:** execute_action signature stable; ActionRegistry additive.
- **Breaking-change protocol:** Major version bump.
- **Deprecation window:** Minimum 2 minor releases.

## Contract Tests Needed

- [ ] Engine executes node via runtime; result flows back
- [ ] ActionNotFound when key not in registry
- [ ] DataLimitExceeded when output exceeds policy
- [ ] NodeStarted/NodeCompleted/NodeFailed emitted in order
- [ ] actions_executed_total, actions_failed_total, action_duration_seconds recorded
