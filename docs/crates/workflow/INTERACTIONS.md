# Interactions

## Ecosystem Map

**nebula-workflow** defines workflow structure and validation. It depends only on **nebula-core** and **petgraph**; no other nebula-* crates.

### Upstream (workflow depends on)

- **nebula-core** — `WorkflowId`, `NodeId`, `ActionId`, `Version`, `InterfaceVersion` for definition and node types.
- **petgraph** — `DependencyGraph` is built on `petgraph::graph::DiGraph<NodeId, Connection>`; topological sort and level computation.
- **serde, serde_json, thiserror, chrono** — serialization and timestamps.

### Downstream (depend on nebula-workflow)

- **nebula-engine** — loads workflow by ID, builds DependencyGraph, schedules nodes in topological order.
- **nebula-execution** — execution layer consumes workflow definition and graph for run lifecycle.
- **nebula-sdk** — re-exports or uses workflow types for authoring and testing.

### Planned / indirect

- **storage** — will persist WorkflowDefinition; schema stability is a contract.
- **api** — will expose GET/POST workflow; same WorkflowDefinition schema.

## Downstream Consumers

- **engine:** Expects `WorkflowDefinition`, `DependencyGraph::from_definition`, `validate_workflow`, `topological_sort`, `compute_levels`. Uses workflow as data; no execution in workflow crate.
- **execution / sdk:** Consume workflow types for execution context and authoring.

## Upstream Dependencies

- **core:** IDs and Version types; required.
- **petgraph:** Graph algorithms; required for DAG operations.
- No fallback: workflow is a leaf for nebula-* (only core).

## Interaction Matrix

| This crate ↔ Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|--------------------|-----------|----------|------------|------------------|-------|
| workflow ↔ core | out | WorkflowId, NodeId, ActionId, Version, InterfaceVersion | sync | N/A | type usage only |
| engine ↔ workflow | in | WorkflowDefinition, DependencyGraph, validate_workflow | sync | Vec&lt;WorkflowError&gt; | engine loads and validates |
| execution ↔ workflow | in | workflow types for run context | sync | N/A | execution consumes definition |
| sdk ↔ workflow | in | WorkflowBuilder, definition types | sync | N/A | authoring |
| storage (planned) ↔ workflow | in | WorkflowDefinition serialized form | sync | N/A | schema stability |

## Runtime Sequence

1. Engine or API loads workflow (by ID from storage or from request body).
2. `validate_workflow(definition)` returns `Vec<WorkflowError>`; caller rejects if non-empty.
3. `DependencyGraph::from_definition(definition)` builds graph; `topological_sort()` / `compute_levels()` for scheduling.
4. Engine executes nodes in order; workflow crate is not involved in execution.

## Cross-Crate Ownership

- **workflow** owns: definition shape, DAG structure, validation rules, WorkflowError variants.
- **engine** owns: loading, scheduling, execution.
- **storage** (when present) owns: persistence; workflow defines serialized shape.

## Versioning and Compatibility

- WorkflowDefinition schema is stable; patch/minor do not break serialized form.
- Breaking schema or validation contract = major + MIGRATION.md.
