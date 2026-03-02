# Architecture

## Problem Statement

- **Business problem:** The engine and API need a shared, serializable model for execution state, result, and transitions so that persistence, queries, and resume behave consistently. Without a single type authority, engine and storage would diverge.
- **Technical problem:** How to define execution-level and node-level state machines with validated transitions, output and journal types, and idempotency keys without pulling in orchestration or persistence.

## Current Architecture

### Module Map

| Module | File | Responsibility |
|--------|------|----------------|
| `status` | `status.rs` | `ExecutionStatus` enum (8 states); `is_terminal`, `is_active`, `is_success`, `is_failure` |
| `state` | `state.rs` | `ExecutionState`, `NodeExecutionState`; per-node attempts and output; `transition_status`, `transition_to` with validation |
| `transition` | `transition.rs` | `can_transition_execution`, `validate_execution_transition`, `can_transition_node`, `validate_node_transition` |
| `output` | `output.rs` | `ExecutionOutput` (Inline / BlobRef), `NodeOutput` (data + status, produced_at, duration, bytes) |
| `attempt` | `attempt.rs` | `NodeAttempt` (attempt_number, idempotency_key, started_at, completed_at, output, error); complete_success / complete_failure |
| `plan` | `plan.rs` | `ExecutionPlan` from `WorkflowDefinition`; parallel_groups, entry_nodes, exit_nodes; `ExecutionBudget` |
| `context` | `context.rs` | `ExecutionContext` (execution_id, budget); `ExecutionBudget` (max_concurrent_nodes) |
| `journal` | `journal.rs` | `JournalEntry` enum (ExecutionStarted, NodeScheduled, NodeStarted, NodeCompleted, NodeFailed, NodeSkipped, …) |
| `idempotency` | `idempotency.rs` | `IdempotencyKey::generate(execution_id, node_id, attempt)`; `IdempotencyManager` (in-memory HashSet check_and_mark) |
| `error` | `error.rs` | `ExecutionError`: InvalidTransition, NodeNotFound, PlanValidation, BudgetExceeded, DuplicateIdempotencyKey, Serialization, Cancelled |

### Data/Control Flow

- **Data:** Engine creates `ExecutionState` and `ExecutionPlan`; updates state via `transition_status` and node `transition_to`. Output flows into `NodeOutput` / `ExecutionOutput`; journal entries are appended by engine. Idempotency keys are generated per attempt and checked by engine/runtime before run.
- **Control:** This crate is passive. Engine (or runtime) calls transition validators and state mutators; execution crate does not schedule or persist. Dependencies: `nebula_core` (ids), `nebula_workflow` (NodeState, DependencyGraph, WorkflowDefinition).

### Key Invariants

- **Execution transitions:** Only defined pairs (e.g. Created→Running, Running→Completed/Failed/Paused/Cancelling/TimedOut, Paused→Running, Cancelling→Cancelled/Failed). Invalid transition returns `ExecutionError::InvalidTransition`.
- **Node transitions:** Follow `NodeState` from nebula_workflow (Pending→Ready→Running→Completed/Failed, Failed→Retrying→Running, etc.). Validated in `validate_node_transition`.
- **Idempotency key:** `{execution_id}:{node_id}:{attempt}`; deterministic for same inputs. `IdempotencyManager::check_and_mark` returns true only on first see.
- **Serialization:** All public state and output types are `Serialize`/`Deserialize`; `ExecutionStatus` uses `#[serde(rename_all = "snake_case")]`.

### Execution Plan, Ephemeral Nodes, and Patches

Execution is often richer than the design-time DAG: the engine may need to insert retry delays, waits on external resources, or other recovery steps that were not drawn on the canvas. `nebula-execution` is the home for this execution-time vocabulary.

- **ExecutionPlan** represents the schedulable view of a workflow, derived from `WorkflowDefinition` and `DependencyGraph`.
- **Ephemeral nodes** (e.g. retry attempts, backoff timers, resource gates) are modeled at execution time via plan/journal types, not by mutating the workflow definition.
- **JournalEntry** is the append-only log that captures both user-node lifecycle and system steps (e.g. NodeScheduled, NodeCompleted, NodeFailed, NodeRetried, NodeWaitingOnResource).
- A future `ExecutionPatch` / recovery-step model will live here as *data* (e.g. "insert wait-on-resource before retry"), so that:
  - engine can extend the execution plan deterministically for a given history and policy;
  - Execution View UIs can render "phantom" or system nodes in timelines without changing stored workflows;
  - durable execution and replay can reconstruct the same extended plan from state + journal.

`nebula-execution` does not decide *when* to add ephemeral steps (that belongs to engine + resilience policy), but it defines the types that describe those steps and their effect on the execution graph.

### Known Bottlenecks

- `IdempotencyManager` is in-memory; no TTL or persistence — duplicate detection is process-local.
- `ExecutionContext` is minimal (execution_id + budget); no credential/resource handles or resume token yet.

## Target Architecture

- **Target module map:** Same structure; add optional resume token type and document schema stability for API.
- **Public contract boundaries:** ExecutionStatus, ExecutionState, NodeExecutionState, ExecutionPlan, ExecutionContext, JournalEntry, NodeOutput, ExecutionOutput, NodeAttempt, IdempotencyKey, IdempotencyManager, ExecutionError, transition validators.
- **Internal invariants:** No persistence in crate; transition rules remain the single source of truth; serialized form stable in patch/minor.

## Design Reasoning

- **State machine in crate, transitions validated here:** Engine owns when to transition; execution crate owns allowed transitions and state shape. Prevents drift and duplicate logic.
- **Idempotency key format in execution crate:** Key is execution-scoped; format is stable so that nebula-idempotency or storage can persist without redefining.
- **No engine or storage dependency:** Execution is vocabulary and validation only; engine persists state using these types.

### Rejected Alternatives

- **Engine owns transition rules:** Rejected. Would duplicate state machine in engine and execution; API and storage would depend on engine for state shape.
- **Persistence in execution crate:** Rejected. Storage abstraction and backends belong in nebula-storage; execution defines types only.

## Comparative Analysis

| Pattern | Decision | Rationale |
|---------|----------|-----------|
| Explicit status enum (8 states) | **Adopt** | Matches Temporal/n8n-style execution lifecycle; terminal vs active is clear |
| Validated transitions | **Adopt** | Prevents invalid state in persistence and API |
| Idempotency key per attempt | **Adopt** | Aligns with at-most-once execution and worker retry |
| Journal as enum | **Adopt** | Audit and replay; serializable events |

## Open Questions

- Q1: Resume token type and semantics for suspend/resume (when execution status is Paused or waiting for trigger).
- Q2: Formal schema snapshot (e.g. JSON fixtures) for ExecutionState and NodeOutput for API compatibility tests.
