# ADR-0056: Type-safe DAG validation (experimental)

**Status:** Proposed (2026-05-14)
**Tags:** workflow, types, dag, experimental

## Context

Charter F1: *"Compiles or doesn't run. Workflow validation happens at
compile time wherever possible."*

Current workflows are defined as YAML and validated at load time.
Errors like "node B's input type does not match node A's output type"
surface at runtime. This is significant DX gap vs. Rust's general
type-system culture.

Inspiration: Servo's typed-CSS pipeline, DataFusion's typed logical
plans, Haskell-style `Stream<A> >>> Stream<B>` composition.

## Decision

Introduce **experimental** `nebula-workflow-typed` crate — workflow
defined as Rust types, connections type-checked at compile time. YAML
workflows continue to work; typed crate is opt-in alternative.

```rust
use nebula_workflow_typed::{Workflow, Connect};

// Author defines workflow as a type:
type DataPipeline = Workflow<
    Connect<
        FetchUsersAction,         // Output = Vec<User>
        TransformAction,          // Input = Vec<User>, Output = Vec<EnrichedUser>
    >,
    Connect<
        TransformAction,          // Output = Vec<EnrichedUser>
        StoreToS3Action,          // Input = Vec<EnrichedUser>
    >,
>;
```

`Connect<NodeA, NodeB>` requires `NodeA::Output: Into<NodeB::Input>`
— compile error otherwise:

```text
error[E0277]: the trait bound `Vec<User>: Into<Vec<Order>>` is not satisfied
   = note: required by `Connect<FetchUsersAction, ProcessOrdersAction>`
   help: cannot connect node producing `Vec<User>` to node consuming `Vec<Order>`
```

Engine compiles `Workflow<...>` type into runtime `WorkflowDefinition`
— same engine runs both YAML-defined and type-defined workflows.

## Consequences

### Positive

- Compile-time graph validation — no runtime "incompatible
  connection" errors for typed workflows.
- IDE autocomplete shows valid downstream nodes (filtered by
  `Output → Input` compatibility).
- Refactor-safe: rename action's `Output` type, compiler catches all
  workflows depending on it.
- Forward-compat with capability system (ADR-0054) — capability
  bounds checkable at workflow level.

### Negative

- Author must learn type-state pattern (`Connect<A, B>`,
  `Workflow<...>` shape).
- Long type chains for big workflows — error messages get noisy.
  Mitigated by type aliases in author's local module.
- Cannot represent runtime-determined topology (LLM-built dynamic DAG
  for AI agents) — typed workflows are static. AI agents stay on
  YAML / programmatic definition.

### Neutral

- Both representations coexist. Typed = opt-in for static workflows
  with strong typing payoff. YAML = default for flexibility.
- Editor visual representation works for both — typed compiles to
  same `WorkflowDefinition`, editor reads that.

## Implementation phases

### Phase 1 (experimental, Q1 2027)

- `Connect<A, B>` type with `A::Output: Into<B::Input>` bound.
- `Workflow<...>` macro to chain `Connect`'s.
- Compile-to-`WorkflowDefinition` conversion.
- Documentation + simple examples.

### Phase 2 (typed parallel branches, Q2 2027)

- `Parallel<(A, B, C)>` type for fan-out.
- `Switch<(Branch1, Branch2)>` for control flow.
- Generic over branch tuple sizes (const-generic).

### Phase 3 (typed slot bindings, Q3 2027)

- Workflow-level type parameters bind resources/credentials at
  type-check time:
  
  ```rust
  type ProductionPipeline = DataPipeline<
      bot = ProductionBot,
      auth = MainBotToken,
  >;
  ```

- Replaces UI picker with compile-time generic bound (per F18 — same
  three-layer architecture, layer 2 becomes generic param).

## References

- Conference Day 3 morning (CONFERENCE-NOTES.md) — F1 principle.
- DataFusion logical plan types.
- Servo CSS pipeline.

## Out of scope

- Visual editor for typed workflows — generates Rust source via
  `quote!`-like emit, not runtime YAML. Deferred.
- Migration tooling YAML → typed — author rewrites; not enough payoff
  to automate in v1.x.
