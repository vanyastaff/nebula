# nebula-runtime

Action execution orchestration for the Nebula workflow engine.

## Crate boundaries

| Crate | Responsibility |
|-------|-----------------|
| **nebula-execution** | State and model only: execution state machine, plan, journal; no orchestration, no action execution. |
| **nebula-action** | Action contract: traits (StatelessAction, etc.), ActionContext/TriggerContext, ActionResult, ActionError. |
| **nebula-runtime** (this crate) | Action execution: registry lookup, run via sandbox, data limits, telemetry; one node at a time. |
| **nebula-engine** | DAG orchestration: builds state/plan, applies transitions, persists; calls runtime to run nodes. |

Context: `execute_action` currently takes **NodeContext** (deprecated in action); target is **ActionContext** / `&impl Context`.

## Scope

- **In scope:**
  - **runtime** — `ActionRuntime` (registry, sandbox: Arc&lt;dyn SandboxRunner&gt;, data_policy, event_bus, metrics); `execute_action(action_key, input, context)` → `Result<ActionResult<Value>, RuntimeError>` (context type currently NodeContext)
  - **registry** — `ActionRegistry` (DashMap key → Arc&lt;dyn InternalHandler&gt; from nebula-plugin); `register()`, `get()`
  - **data_policy** — `DataPassingPolicy` (max_node_output_bytes, max_total_execution_bytes, large_data_strategy), `LargeDataStrategy` (Reject, SpillToBlob)
  - **error** — `RuntimeError` (ActionNotFound, ActionError, DataLimitExceeded, Internal)
  - Telemetry: EventBus (NodeStarted, NodeCompleted, NodeFailed), MetricsRegistry

- **Out of scope:**
  - Workflow scheduling (see `nebula-engine`)
  - Trigger type definitions (see `nebula-action` — webhook, schedule, etc.)
  - Sandbox implementation (this crate: SandboxRunner, InProcessSandbox)
  - Action definitions (see `nebula-action`, `nebula-plugin`)

## Current State

- **Maturity:** MVP — ActionRuntime, ActionRegistry, DataPassingPolicy; engine integration complete
- **Key strengths:** Clean separation from engine; sandbox and queue in-crate; data limit enforcement; telemetry events and metrics
- **Key risks:** Isolation level logic TODO (all actions run directly); SpillToBlob not implemented; no trigger lifecycle orchestration yet

## Target State

- **Production criteria:** Isolation level routing (trusted vs sandboxed); SpillToBlob for large outputs; trigger lifecycle (trigger types in nebula-action)
- **Compatibility guarantees:** ActionRuntime::execute_action signature stable; ActionRegistry API additive-only

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [MIGRATION.md](./MIGRATION.md)
