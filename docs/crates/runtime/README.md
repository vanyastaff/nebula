# nebula-runtime

Action execution orchestration for the Nebula workflow engine.

## Scope

- **In scope:**
  - **runtime** — `ActionRuntime` (registry, sandbox: Arc&lt;dyn SandboxRunner&gt;, data_policy, event_bus, metrics); `execute_action(action_key, input, NodeContext)` → `Result<ActionResult<Value>, RuntimeError>`
  - **registry** — `ActionRegistry` (DashMap key → Arc&lt;dyn InternalHandler&gt;); `register()`, `get()`
  - **data_policy** — `DataPassingPolicy` (max_node_output_bytes, max_total_execution_bytes, large_data_strategy), `LargeDataStrategy` (Reject, SpillToBlob)
  - **error** — `RuntimeError` (ActionNotFound, ActionError, DataLimitExceeded, Internal)
  - Telemetry: EventBus (NodeStarted, NodeCompleted, NodeFailed), MetricsRegistry

- **Out of scope:**
  - Workflow scheduling (see `nebula-engine`)
  - Trigger type definitions (see `nebula-action` — webhook, schedule, etc.)
  - Sandbox implementation (see `nebula-ports`, `nebula-sandbox-inprocess`)
  - Action definitions (see `nebula-action`, `nebula-plugin`)

## Current State

- **Maturity:** MVP — ActionRuntime, ActionRegistry, DataPassingPolicy; engine integration complete
- **Key strengths:** Clean separation from engine; sandbox abstraction via ports; data limit enforcement; telemetry events and metrics
- **Key risks:** Isolation level logic TODO (all actions run directly); SpillToBlob not implemented; no trigger lifecycle orchestration yet

## Target State

- **Production criteria:** Isolation level routing (trusted vs sandboxed); SpillToBlob for large outputs; trigger lifecycle (trigger types in nebula-action)
- **Compatibility guarantees:** ActionRuntime::execute_action signature stable; ActionRegistry API additive-only

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
