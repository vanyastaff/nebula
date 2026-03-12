# Architecture

## Crate boundaries

Runtime sits between **nebula-engine** (orchestration) and **nebula-action** / **nebula-plugin** (handler contract). Execution state and plan live in **nebula-execution**; runtime does not depend on execution.

## Problem Statement

- **Business problem:** Workflow nodes execute actions (HTTP, DB, transforms). The engine schedules nodes; something must resolve action handlers, run them (with optional isolation), enforce data limits, and emit observability events.
- **Technical problem:** Provide ActionRuntime that bridges engine → action execution, uses SandboxRunner for isolation, enforces DataPassingPolicy, and integrates with telemetry.

## Current Architecture

### Module Map

| Location | Responsibility |
|----------|----------------|
| `runtime.rs` | ActionRuntime — registry lookup, sandbox execution, data policy, telemetry |
| `registry.rs` | ActionRegistry — DashMap of InternalHandler by key |
| `data_policy.rs` | DataPassingPolicy, LargeDataStrategy, check_output_size |
| `error.rs` | RuntimeError — ActionNotFound, ActionError, DataLimitExceeded, Internal |

### Data/Control Flow

1. **Engine** creates NodeTask with `Arc<ActionRuntime>`, calls `runtime.execute_action(action_key, input, context)` (context type: NodeContext today; target ActionContext).
2. **ActionRuntime** looks up handler via `registry.get(action_key)`.
3. **ActionRuntime** emits NodeStarted, records start time.
4. **ActionRuntime** calls `handler.execute(input, context)` (TODO: route via sandbox for IsolationLevel::CapabilityGated/Isolated).
5. **ActionRuntime** enforces `data_policy.check_output_size` on primary output.
6. **ActionRuntime** emits NodeCompleted or NodeFailed, records metrics (actions_executed_total, actions_failed_total, action_duration_seconds).

### Known Bottlenecks

- **Isolation bypass:** TODO in runtime — all actions run directly; no SandboxedContext or capability checks yet.
- **SpillToBlob:** LargeDataStrategy::SpillToBlob logs warning but does not spill; Reject is the only working path.
- **max_total_execution_bytes:** Defined in policy but not enforced (no cross-node aggregation).

## Target Architecture

### Target Module Map

```
nebula-runtime/
├── runtime.rs      — ActionRuntime (current)
├── registry.rs     — ActionRegistry (current)
├── data_policy.rs  — DataPassingPolicy (current)
├── error.rs        — RuntimeError (current)
├── isolation.rs    — (Phase 2) resolve_isolation_level, SandboxedContext
├── trigger/        — (Phase 2) Trigger lifecycle orchestration; trigger types in nebula-action
└── coordination/  — (Phase 3) WorkflowCoordinator, RuntimeRegistry
```

### Public Contract Boundaries

- `ActionRuntime::execute_action(action_key, input, context)` → `Result<ActionResult, RuntimeError>`
- `ActionRegistry::get(key)` → `Result<Arc<dyn InternalHandler>, RuntimeError>`
- `DataPassingPolicy::check_output_size(output)` → `Result<u64, (u64, u64)>`
- SandboxRunner trait and InProcessSandbox in this crate

### Internal Invariants

- Emit NodeStarted before execute; NodeCompleted/NodeFailed after.
- Data limit check runs after successful execution, before returning.
- Metrics recorded regardless of success/failure.

## Design Reasoning

### Key Trade-off 1: Direct vs sandbox execution

- **Current:** All actions run via `handler.execute()` directly; sandbox is injected but isolation logic is TODO.
- **Target:** IsolationLevel::None → direct; CapabilityGated/Isolated → SandboxedContext + sandbox.execute().
- **Consequence:** ActionMetadata needs isolation_level/capabilities; runtime needs resolve_isolation_level().

### Key Trade-off 2: Data limit enforcement

- **Current:** Per-node output only; Reject or SpillToBlob (unimplemented).
- **Target:** SpillToBlob writes to blob storage, returns reference; max_total_execution_bytes tracked across nodes.
- **Consequence:** Blob storage abstraction; execution-scoped size accumulator.

### Rejected Alternatives

- **No data limits:** OOM risk for large workflows.
- **Engine-owned execution:** Runtime provides clearer boundary; engine focuses on scheduling.

## Comparative Analysis

Sources: n8n, Node-RED, Temporal, Prefect.

| Pattern | Verdict | Rationale |
|---------|---------|-----------|
| Registry of handlers by key | **Adopt** | n8n, Node-RED; simple lookup |
| Sandbox/isolation for untrusted code | **Adopt** | Temporal; capability-gated access |
| Data size limits | **Adopt** | Prefect; prevent OOM |
| Spill to blob for large data | **Adopt** | Phase 2; reference passing |
| Triggers in nebula-action | **Adopt** | Trigger types (webhook, schedule) are actions; no separate crate |

## Breaking Changes (if any)

- None planned; isolation and SpillToBlob are additive.

## Open Questions

- Q1: Trigger lifecycle orchestration — in runtime or engine? (Trigger types live in nebula-action)
- Q2: Who owns max_total_execution_bytes tracking — runtime or engine?
