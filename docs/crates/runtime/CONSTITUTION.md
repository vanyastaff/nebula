# nebula-runtime Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

The engine decides *which* node runs next; something must *run* it: resolve the action, build context (credentials, resources), invoke the action through the sandbox, enforce data limits, and report result and telemetry. That layer is the runtime.

**nebula-runtime is the action execution orchestration layer.**

It answers: *Given a node and execution context, how does the platform resolve the action, supply credentials and resources, execute it (with isolation and data limits), and return ActionResult?*

```
Engine (or caller) invokes runtime.execute_action(node, context, input)
    ↓
ActionRegistry looks up action by key; runtime builds Context (credentials, resources, scope)
    ↓
Sandbox (via ports) runs action with capability-checked context; DataPassingPolicy enforced
    ↓
Returns ActionResult; emits NodeStarted / NodeCompleted / NodeFailed for telemetry
```

This is the runtime contract: it bridges engine and action/sandbox; it does not define workflow DAG or action implementations.

---

## User Stories

### Story 1 — Engine Runs a Single Node (P1)

Engine has scheduled a node. It calls runtime with node config, execution context, and input. Runtime resolves the action, runs it, and returns the action result. Engine interprets result (continue, wait, retry, fail).

**Acceptance**:
- `execute_action(node, context, input)` returns `Result<ActionResult, RuntimeError>`
- ActionRegistry resolves by plugin/key; ActionNotFound if missing
- Data limits (max output size, reject or spill) enforced; RuntimeError::DataLimitExceeded when exceeded

### Story 2 — Action Receives Credentials and Resources (P1)

Action declared CredentialRef and ResourceRef in ActionComponents. Runtime builds context that supplies only those; sandbox enforces that the action cannot access undeclared credentials or resources.

**Acceptance**:
- Context implements or bridges to credential and resource access
- Sandbox (inprocess or future isolate) receives capability-checked proxy
- Violations produce explicit error, not silent denial

### Story 3 — Telemetry and Metrics Without Blocking (P2)

Runtime emits NodeStarted, NodeCompleted, NodeFailed and action metrics. Slow or failing subscribers do not block execution.

**Acceptance**:
- Telemetry integration is fire-and-forget
- RuntimeError variants (ActionNotFound, ActionError, DataLimitExceeded) allow classification
- Metrics (latency, success/failure) optional via feature or adapter

---

## Core Principles

### I. Runtime Bridges Engine and Action/Sandbox

**Runtime does not own workflow graph or action definitions; it executes one node at a time with context.**

**Rationale**: Engine owns DAG and scheduling; action crate owns contract. Runtime is the glue: registry lookup, context building, sandbox invocation, data limits.

**Rules**:
- No workflow or DAG types in runtime public API for scheduling
- ActionRegistry is key → action factory or instance; runtime calls execute

### II. Context Is Built by Runtime (From Engine Handoff)

**Execution context (execution_id, workflow_id, credentials, resources) is provided by engine or runtime; runtime fills in credential and resource access.**

**Rationale**: Engine owns execution lifecycle; credential and resource managers are injected. Runtime builds the concrete Context that actions see.

**Rules**:
- Context type is defined in action crate (trait or bridge); runtime implements or provides it
- Credential and resource access are scoped to execution/workflow

### III. Data Limits Are Enforced

**Output size and strategy (reject vs spill to blob) are enforced so that one node cannot unboundedly consume memory or storage.**

**Rationale**: Untrusted or buggy actions must not OOM the process. DataPassingPolicy is part of the contract.

**Rules**:
- DataLimitExceeded is a distinct RuntimeError variant
- Policy (max size, Reject/SpillToBlob) is configurable; SpillToBlob implementation may be phased

### IV. Observability Must Not Block Execution

**Telemetry and metrics are best-effort. Lagging subscribers or hook failures must not cancel or delay the execute path.**

**Rationale**: Same as engine: observability failures must not become workflow failures.

**Rules**:
- Event emit is non-blocking
- Hook or metrics failure is logged; execute still returns action result

---

## Production Vision

### The runtime in an n8n-class fleet

In production, runtime runs inside the same process as the engine (or worker). ActionRuntime holds ActionRegistry and sandbox abstraction; it receives execution context and input, looks up action, builds Context (credentials, resources), runs action through sandbox, applies data limits, and returns ActionResult. Telemetry events go to EventBus or metrics.

```
runtime.rs    — ActionRuntime: registry (ActionRegistry), sandbox (Arc<dyn SandboxRunner>), data_policy (DataPassingPolicy), event_bus (EventBus), metrics (MetricsRegistry)
                 execute_action(action_key, input, NodeContext) → Result<ActionResult<Value>, RuntimeError>
registry.rs   — ActionRegistry: DashMap<String, Arc<dyn InternalHandler>>; register(handler), get(key)
data_policy.rs — DataPassingPolicy (max_node_output_bytes, max_total_execution_bytes, large_data_strategy: LargeDataStrategy); LargeDataStrategy::Reject | SpillToBlob
error.rs      — RuntimeError: ActionNotFound { key }, ActionError(ActionError), DataLimitExceeded { limit_bytes, actual_bytes }, Internal(String)
```

SandboxRunner (nebula-ports) is the sandbox abstraction; InternalHandler (nebula-plugin) is the action contract. Isolation level routing (trusted vs sandboxed) is TODO; SpillToBlob is Phase 2. Telemetry: EventBus.emit(NodeStarted / NodeCompleted / NodeFailed), MetricsRegistry counter/histogram.

### From the archives: node execution and runtime role

The archive (`docs/crates/runtime/_archive/`: archive-node-execution.md, archive-crates-dependencies.md, archive-layers-interaction.md, from-archive/, from-core-full/) describes executor and runtime as the layer that runs nodes. Production vision: runtime remains the single place that executes an action per node; engine only schedules and interprets results.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Isolation level routing (trusted vs sandboxed) | High | Route to different sandbox impls |
| SpillToBlob implementation | High | Large outputs to blob store + reference |
| Trigger lifecycle (start/stop) | Medium | Trigger types; runtime may run trigger lifecycle |
| Context concrete types (ActionContext/TriggerContext) | Medium | Replace bridge with capability modules |

---

## Key Decisions

### D-001: ActionRegistry in Runtime, Not Engine

**Decision**: Runtime owns the registry that maps action key to executable action. Engine passes node config; runtime resolves.

**Rationale**: Engine stays agnostic to action registration and plugin loading. Runtime is the natural place for "how to run an action by key."

**Rejected**: Engine holding registry — would mix orchestration and action resolution.

### D-002: Sandbox Behind Ports/Abstraction

**Decision**: Runtime calls sandbox through an abstraction (ports); concrete sandbox (inprocess, isolate, etc.) is pluggable.

**Rationale**: Allows different isolation levels and future sandbox implementations without changing runtime API.

**Rejected**: Runtime depending on single sandbox crate — would block alternative sandboxes.

### D-003: DataPassingPolicy as Part of Contract

**Decision**: Max output size and strategy (Reject, SpillToBlob) are part of runtime/engine contract.

**Rationale**: Prevents unbounded memory use; spill enables large payloads without OOM.

**Rejected**: No limit — unacceptable for multi-tenant or untrusted actions.

---

## Open Proposals

### P-001: ActionContext and TriggerContext in Runtime

**Problem**: NodeContext is a bridge; target is explicit capability structs.

**Proposal**: Runtime builds ActionContext/TriggerContext (from action crate) with ResourceAccessor, CredentialAccessor, etc.

**Impact**: Requires action crate to define concrete types; runtime implements builders.

### P-002: SpillToBlob Backend

**Problem**: Large outputs need storage; currently only Reject is fully implemented.

**Proposal**: Define blob storage interface; runtime writes oversize output to blob store and returns reference; engine/expression resolve reference when needed.

**Impact**: New dependency or trait; storage/engine contract for blob refs.

---

## Non-Negotiables

1. **Runtime executes one node at a time** — no DAG or scheduling in runtime.
2. **Context is built by runtime** — credentials and resources supplied per ActionComponents.
3. **Data limits enforced** — DataLimitExceeded when policy exceeded.
4. **Observability does not block** — fire-and-forget events and metrics.
5. **ActionRegistry is the single lookup** — engine does not resolve action by key.
6. **Breaking execute_action or context = major + MIGRATION.md** — engine and actions depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to execute_action or RuntimeError.
- **MINOR**: Additive (new options, new telemetry fields). No removal.
- **MAJOR**: Breaking changes to execution or context. Requires MIGRATION.md.
