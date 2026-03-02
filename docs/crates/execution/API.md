# API

## Public Surface

### Stable APIs

- **Status and state:** `ExecutionStatus`, `ExecutionState`, `NodeExecutionState` — state machine and per-node state; `transition_status`, `transition_to` with validation.
- **Plan and context:** `ExecutionPlan::from_workflow`, `ExecutionContext`, `ExecutionBudget` — plan from workflow definition; lightweight context for runtime.
- **Output:** `ExecutionOutput` (Inline, BlobRef), `NodeOutput` — materialized output for persistence and inter-node transport.
- **Attempt and idempotency:** `NodeAttempt`, `IdempotencyKey::generate`, `IdempotencyManager` — attempt tracking and duplicate detection.
- **Journal:** `JournalEntry` — audit events (ExecutionStarted, NodeScheduled, NodeStarted, NodeCompleted, NodeFailed, NodeSkipped, …).
- **Transition validation:** `validate_execution_transition`, `validate_node_transition`, `can_transition_execution`, `can_transition_node` — used by engine before applying transitions.
- **Error:** `ExecutionError` — InvalidTransition, NodeNotFound, PlanValidation, BudgetExceeded, DuplicateIdempotencyKey, Serialization, Cancelled.

### Experimental / Evolving

- `ExecutionContext` — currently minimal (execution_id, budget); may gain resume token or capability handles in future.
- `IdempotencyManager` — in-memory only; persistent backend is out of scope (see nebula-idempotency).

### Hidden / Internal

- `serde_duration_opt` — crate-internal serde helper for `Option<Duration>`.

## Usage Patterns

1. **Engine creates execution:** `ExecutionState::new(execution_id, workflow_id, node_ids)`; `ExecutionPlan::from_workflow(execution_id, workflow, budget)`.
2. **Engine applies transitions:** `state.transition_status(ExecutionStatus::Running)`; for each node `node_state.transition_to(NodeState::Ready)` then `Running` then `Completed`/`Failed`. Invalid transition returns `ExecutionError`.
3. **Engine records output:** `NodeOutput::inline(value, NodeState::Completed, bytes)` or `blob_ref(...)`; attach to `NodeExecutionState.current_output` and push `NodeAttempt` with `complete_success`/`complete_failure`.
4. **Idempotency before run:** `IdempotencyKey::generate(execution_id, node_id, attempt_number)`; `idempotency_manager.check_and_mark(&key)` — if false, skip run and return cached result or DuplicateIdempotencyKey.
5. **Journal:** Append `JournalEntry` variants as execution progresses; engine or storage persists journal separately.

## Minimal Example

```rust
use nebula_execution::{
    ExecutionState, ExecutionStatus, NodeExecutionState,
    ExecutionPlan, ExecutionContext, ExecutionBudget,
    IdempotencyKey, IdempotencyManager, NodeAttempt,
    validate_execution_transition,
};
use nebula_core::{ExecutionId, NodeId, WorkflowId};
use nebula_workflow::NodeState;

// Create execution state and plan (engine would load workflow)
let execution_id = ExecutionId::new();
let workflow_id = WorkflowId::new();
let node_ids = [NodeId::new(), NodeId::new()];
let state = ExecutionState::new(execution_id, workflow_id, &node_ids);
assert_eq!(state.status, ExecutionStatus::Created);

// Valid transition
let mut state = state;
state.transition_status(ExecutionStatus::Running).unwrap();
assert!(state.started_at.is_some());

// Idempotency
let key = IdempotencyKey::generate(execution_id, node_ids[0], 0);
let mut mgr = IdempotencyManager::new();
assert!(mgr.check_and_mark(&key));   // first time: new
assert!(!mgr.check_and_mark(&key)); // second time: duplicate
```

## Error Semantics

- **Retryable:** None in this crate (execution crate does not perform I/O). Callers may retry on `PlanValidation` if workflow definition is fixed.
- **Fatal:** `InvalidTransition`, `NodeNotFound`, `DuplicateIdempotencyKey` — do not retry same transition or same key. `Cancelled` — execution was cancelled.
- **Validation:** `PlanValidation` — workflow or plan invalid. `Serialization` — serde error when serializing/deserializing state or output.

## Compatibility Rules

- **Major version bump:** Changing `ExecutionStatus` or `ExecutionState`/`NodeExecutionState` field semantics or removing variants; changing transition rules; changing `IdempotencyKey` format; changing `ExecutionOutput`/`NodeOutput` or `JournalEntry` schema.
- **Deprecation policy:** Deprecate in minor with rustdoc and optional feature; remove in next major. Serialized form: patch/minor do not change JSON shape for state, output, or journal.
