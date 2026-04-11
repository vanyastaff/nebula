# nebula-workflow
Workflow definition types and DAG model — pure data, no execution.

## Invariants
- Definition types only. No execution state, no scheduling logic.
- Workflows must be DAGs. Cyclic connections fail `validate_workflow`.
- `owner_id` is `Option<OwnerId>` for backward compat with existing serialized workflows.
- `UiMetadata` is opaque to the engine — only desktop/web editor reads it.

## Key Decisions
- `validate_workflow` collects all errors (not fail-fast).
- `PartialEq` derived on all definition types.

## Traps
- `NodeDefinition::new` validates `ActionKey` — returns `Result`.
- `ParamValue` (definition-time) vs `FieldValues` (runtime, nebula-parameter) — different types.
- `NodeState` vs `NodeExecutionState` (nebula-execution) — different types.
- Adding fields to `WorkflowDefinition` requires updating ALL construction sites. Search `WorkflowDefinition {`.
