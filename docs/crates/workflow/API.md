# API

## Public Surface

- **Stable:** `WorkflowDefinition`, `WorkflowConfig`, `NodeDefinition`, `Connection`, `ParamValue`, `DependencyGraph`, `WorkflowBuilder`, `validate_workflow`, `WorkflowError`, `NodeState`, `WorkflowError` variants. Serialized form (JSON) is stable in patch/minor.
- **Experimental:** None; all public types are part of the authoring/engine contract.
- **Hidden/internal:** Graph internals (petgraph); validation helpers.

## Usage Patterns

- **Load and validate:** Caller (engine or API) loads definition from storage or request; calls `validate_workflow(definition)`; if empty errors, builds `DependencyGraph::from_definition(definition)` and uses `topological_sort()` / `compute_levels()` for scheduling.
- **Build programmatically:** Use `WorkflowBuilder` for fluent construction; builder validates on build.

## Minimal Example

```rust
use nebula_workflow::{WorkflowDefinition, validate_workflow, DependencyGraph};

let definition: WorkflowDefinition = serde_json::from_str(json)?;
let errors = validate_workflow(&definition);
if !errors.is_empty() {
    return Err(ValidationFailed(errors));
}
let graph = DependencyGraph::from_definition(&definition)?;
let order = graph.topological_sort(); // for engine scheduling
```

## Error Semantics

- **Validation errors:** `validate_workflow` returns `Vec<WorkflowError>`; variants include EmptyName, NoNodes, DuplicateNodeId, UnknownNode, SelfLoop, CycleDetected, NoEntryNodes, InvalidParameterReference, GraphError. Not retryable; caller must fix definition.
- **Graph construction:** `DependencyGraph::from_definition` may fail if definition is invalid; call validate_workflow first.
- **Fatal:** Invalid DAG (e.g. cycle) must never be executed; validation is the gate.

## Compatibility Rules

- **Major bump:** Breaking change to serialized WorkflowDefinition or to validate_workflow / DependencyGraph contract. MIGRATION.md required.
- **Minor:** Additive only (new optional fields, new error variants that are additive). No removal of fields or variants.
- **Deprecation:** Deprecated fields get at least one minor version with doc and optional migration path before removal (major).
