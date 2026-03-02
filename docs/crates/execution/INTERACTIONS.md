# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|--------------|-------------|
| `nebula-core` | Upstream | `ExecutionId`, `NodeId`, `WorkflowId` — execution crate uses only these IDs |
| `nebula-workflow` | Upstream | `WorkflowDefinition`, `DependencyGraph`, `NodeState` — plan construction and node transition validation |
| `nebula-engine` | Downstream | Consumes `ExecutionState`, `ExecutionPlan`, `ExecutionContext`, `JournalEntry`, `IdempotencyKey`/`IdempotencyManager`; applies transitions and persists state |
| `nebula-runtime` | Downstream | May use `ExecutionContext`, `NodeAttempt`, `IdempotencyKey` when executing a node |
| `nebula-api` | Downstream (planned) | Will return `ExecutionStatus`, execution state, and result for GET /api/v1/executions/:id |
| `nebula-storage` | Indirect | Engine persists `ExecutionState` and journal; storage trait is not in execution crate |

### Planned

- **nebula-idempotency:** Persistent idempotency store; key format is defined here (`IdempotencyKey`); idempotency crate may wrap or replace `IdempotencyManager` with storage-backed implementation.

## Downstream Consumers

- **nebula-engine:** Builds `ExecutionState` and `ExecutionPlan`; calls `transition_status` and node `transition_to`; appends `JournalEntry`; uses `IdempotencyManager::check_and_mark` before running a node; stores state and journal (via its own persistence).
- **nebula-runtime:** Receives `ExecutionContext` and node context; may generate `IdempotencyKey` and check idempotency when executing action.
- **nebula-api:** Will serialize `ExecutionState` (or a view) and `ExecutionStatus` for REST responses; must not break serialized form in patch/minor.

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| nebula-core | ExecutionId, NodeId, WorkflowId | ID types and serde | None |
| nebula-workflow | WorkflowDefinition, DependencyGraph, NodeState | Plan build and node transition rules | None |

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|--------------------|-----------|---------|------------|------------------|-------|
| execution ↔ core | in | Uses ExecutionId, NodeId, WorkflowId | sync | N/A | No fallback |
| execution ↔ workflow | in | Uses WorkflowDefinition, DependencyGraph, NodeState | sync | PlanValidation on graph errors | Plan build can fail |
| engine ↔ execution | out | Uses State, Plan, Context, Journal, IdempotencyKey/Manager, transition validators | sync | InvalidTransition, PlanValidation, DuplicateIdempotencyKey | Engine must handle errors |
| api ↔ execution | out | Serializes ExecutionStatus, state, result for API | sync | N/A | Schema stability required |

## Engine / Runtime Sequence

(Execution crate defines the types; engine and runtime perform the steps.)

1. Engine creates execution: `ExecutionState::new`, `ExecutionPlan::from_workflow`.
2. Engine transitions execution to Running; for each node in plan order, transitions node to Ready then Running.
3. Before running node, engine generates `IdempotencyKey`, calls `IdempotencyManager::check_and_mark`; if duplicate, skip run or return cached result.
4. Runtime executes node; on success/failure, engine updates `NodeExecutionState` (output, attempt, transition to Completed/Failed/Retrying).
5. Engine appends `JournalEntry` events; persists state and journal (engine/storage responsibility).
6. When all nodes terminal, engine transitions execution to Completed/Failed; persists final state.

## Cross-Crate Ownership

- **Execution state and result model:** nebula-execution (this crate).
- **Orchestration and scheduling:** nebula-engine.
- **Persistence of state and journal:** nebula-engine / nebula-storage (execution crate does not persist).
- **Idempotency key format:** nebula-execution. Idempotency persistence: nebula-idempotency (planned) or engine.
- **Transition enforcement:** nebula-execution (validators); engine calls validators before mutating state.

## Failure Propagation

- Transition errors (`ExecutionError::InvalidTransition`) and plan validation errors are returned to engine; engine does not apply invalid transition.
- `DuplicateIdempotencyKey` is returned when idempotency check fails; engine/runtime should return cached result or 409, not retry same key.
- Serialization errors bubble to caller when persisting or loading state/output.

## Versioning and Compatibility

- **Compatibility promise:** Patch/minor preserve public API and serialized form of ExecutionStatus, ExecutionState, NodeExecutionState, ExecutionOutput, NodeOutput, JournalEntry, IdempotencyKey format. Breaking changes only in major with MIGRATION.md.
- **Breaking-change protocol:** Document in MIGRATION.md; deprecation window one minor cycle where feasible.
- **Downstream:** Engine and API must pin execution crate version or accept minor updates; major upgrade requires migration steps.

## Contract Tests Needed

- Serialization roundtrip for ExecutionState, NodeOutput, JournalEntry, ExecutionStatus (already in crate tests).
- Transition matrix: every allowed pair (from, to) succeeds; disallowed pairs return InvalidTransition.
- IdempotencyKey format stability: same (execution_id, node_id, attempt) ⇒ same string.
- Integration: engine builds plan from workflow, applies transitions, persists state (in engine or e2e tests).
