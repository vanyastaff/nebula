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

## Contributor Workflow (Safe Changes)

Use this sequence when changing public workflow types or validation behavior.

1. Change definition/validation code in `crates/workflow`.
2. Add or update tests for the changed contract.
3. Run crate-scoped checks first:

```bash
cargo check -p nebula-workflow
cargo test -p nebula-workflow
```

4. If public shape changed, run workspace checks and update dependent docs.

## Error Semantics

- **Validation errors:** `validate_workflow` returns `Vec<WorkflowError>`; variants include EmptyName, NoNodes, DuplicateNodeId, UnknownNode, SelfLoop, CycleDetected, NoEntryNodes, InvalidParameterReference, GraphError. Not retryable; caller must fix definition.
- **Graph construction:** `DependencyGraph::from_definition` may fail if definition is invalid; call validate_workflow first.
- **Fatal:** Invalid DAG (e.g. cycle) must never be executed; validation is the gate.

## Compatibility Rules

- **Major bump:** Breaking change to serialized WorkflowDefinition or to validate_workflow / DependencyGraph contract. MIGRATION.md required.
- **Minor:** Additive only (new optional fields, new error variants that are additive). No removal of fields or variants.
- **Deprecation:** Deprecated fields get at least one minor version with doc and optional migration path before removal (major).

## Definition of Done for API-Surface Changes

1. `validate_workflow` behavior change is covered by tests.
2. Serialized compatibility impact is explicitly described.
3. `DependencyGraph::from_definition` assumptions remain documented.
4. Any new error variant is explained in Error Semantics.

## Frequent Pitfalls

1. Adding validation rule without updating error semantics docs.
2. Breaking serialized shape by changing required fields.
3. Assuming graph construction performs all validation checks.
4. Shipping behavior changes without migration notes for callers.
