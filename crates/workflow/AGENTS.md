# nebula-workflow — Agent orientation
> Local guide for `crates/workflow/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** The shared, serde-round-trippable `WorkflowDefinition` + `DependencyGraph` (DAG) + activation-time `validate_workflow` that every higher layer (API, engine, storage) imports instead of re-parsing workflow JSON.
**Layer:** Core — depends on `nebula-core`, `nebula-error`, and `nebula-schema`; no action/engine/storage/api imports.

## Key files

- `src/lib.rs` — module wiring + public re-exports (the authoring surface).
- `src/definition.rs` — `WorkflowDefinition`, `WorkflowConfig`, schema-version constants; JSON round-trip seam.
- `src/node.rs` — `NodeDefinition`, `ParamValue` (unresolved expr strings), `RateLimit`, `SlotBinding`.
- `src/connection.rs` — port-driven `Connection` edges (Spec 28); activation contract in module doc.
- `src/graph.rs` — `DependencyGraph`: `petgraph` topo-sort + per-level batching (feeds `ExecutionPlan`).
- `src/validate.rs` — `validate_workflow`, the canon §10 shift-left activation gate.
- `src/resolver.rs` — injected `NodeSchemaResolver` and direction-typed node input/output schemas; catalog implementations stay in higher layers.
- `src/builder.rs` — `WorkflowBuilder` fluent construction.

## Conventions & never-do

- `validate_workflow` returns `Vec<WorkflowError>` (empty means no structural errors), never mutates the definition, and stops graph-dependent checks when no nodes exist. Schema-aware validation has separate resolver/mode entry points. Structural success alone does not prove frozen-registry compilation or runtime admission; trace activation through the API gateway and engine activation service when changing validation.
- Edges carry NO conditions/matchers — conditional + error routing live in `ControlAction` nodes (trait in `nebula_action::control`); failed nodes activate only `from_port == "error"` edges. Do not re-add the removed `EdgeCondition`/`ResultMatcher`/`ErrorMatcher`.
- `ParamValue` holds unresolved expression strings; this crate must NOT evaluate them (that is `nebula-expression`) and must NOT execute/schedule the DAG (that is `nebula-engine`) or persist it (that is `nebula-storage`/`nebula-api`).
- `WorkflowDefinition` MUST survive a `serde_json` round-trip without loss — schema is a public compat surface.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Definition, DAG, and validation | Unit tests in [src/definition.rs](src/definition.rs), [src/graph.rs](src/graph.rs), [src/validate.rs](src/validate.rs), and [src/builder.rs](src/builder.rs). |
| Activation/type-check consequences | Plugin [graph_plan](../plugin/tests/graph_plan.rs) and API [workflow_activation](../api/tests/workflow_activation.rs); local structural validation alone does not prove admission. |

## See also

- `README.md` — full design · [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §10/§12.2 · Spec 28 (port-driven routing).
