# nebula-workflow
Workflow definition types and DAG model — pure data, no execution.

## Invariants
- Contains only definition types. No execution state, no scheduling logic.
- Workflows must be DAGs. Cyclic connections compile fine but fail `validate_workflow`.

## Key Decisions
- `DependencyGraph` wraps `petgraph` — computes topological levels for parallel execution. The engine reads levels from here.
- `WorkflowBuilder` is the fluent construction API with inline validation.
- `validate_workflow` returns multiple errors at once (not fail-fast). Collect all before reporting.
- `EdgeCondition` / `ErrorMatcher` / `ResultMatcher` define conditional branching on connections.

## Traps
- `ParamValue` is the parameter value type at definition time (static). `FieldValues` (from nebula-parameter) is the runtime value map. They are different.
- `NodeState` is lightweight node progress tracking inside `WorkflowDefinition` — not to be confused with `NodeExecutionState` in nebula-execution.
- `RetryConfig` and `CheckpointingConfig` live on `WorkflowConfig` — they apply to the whole workflow, not individual nodes.

## Relations
- Depends on nebula-core (IDs, DependencyGraph primitives). Used by nebula-engine, nebula-storage.

<!-- reviewed: 2026-03-30 — derive Classify migration -->

<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 — dep cleanup only: removed unused Cargo.toml deps via cargo shear --fix, no code changes -->
