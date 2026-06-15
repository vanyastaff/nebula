# nebula-workflow — Agent orientation
> Agent quick-map for `crates/workflow/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** The shared, serde-round-trippable `WorkflowDefinition` + `DependencyGraph` (DAG) + activation-time `validate_workflow` that every higher layer (API, engine, storage) imports instead of re-parsing workflow JSON.
**Layer:** Core — depends only downward (only `nebula-core` + `nebula-error`; no engine/storage/api imports).

## Commands
- `cargo check -p nebula-workflow`
- `cargo nextest run -p nebula-workflow`  ·  doctests: none (`[lib] doctest = false` in Cargo.toml)

## Key files
- `src/lib.rs` — module wiring + public re-exports (the authoring surface).
- `src/definition.rs` — `WorkflowDefinition`, `WorkflowConfig`, schema-version constants; JSON round-trip seam.
- `src/node.rs` — `NodeDefinition`, `ParamValue` (unresolved expr strings), `RateLimit`, `SlotBinding`.
- `src/connection.rs` — port-driven `Connection` edges (Spec 28); activation contract in module doc.
- `src/graph.rs` — `DependencyGraph`: `petgraph` topo-sort + per-level batching (feeds `ExecutionPlan`).
- `src/validate.rs` — `validate_workflow`, the canon §10 shift-left activation gate.
- `src/builder.rs` — `WorkflowBuilder` fluent construction.

## Conventions & never-do
- `validate_workflow` is the canonical activation gate (canon §10 / §12.2): collects ALL errors into a `Result`, never mutates the definition. The *call* is enforced in `nebula-api` handlers — this crate only owns the function.
- Edges carry NO conditions/matchers — conditional + error routing live in `ControlAction` nodes (trait in `nebula_action::control`); failed nodes activate only `from_port == "error"` edges. Do not re-add the removed `EdgeCondition`/`ResultMatcher`/`ErrorMatcher`.
- `ParamValue` holds unresolved expression strings; this crate must NOT evaluate them (that is `nebula-expression`) and must NOT execute/schedule the DAG (that is `nebula-engine`) or persist it (that is `nebula-storage`/`nebula-api`).
- `WorkflowDefinition` MUST survive a `serde_json` round-trip without loss — schema is a public compat surface.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`WorkflowError`; no panicking unwrap/expect/panic in lib code. (4 existing builder-invariant `panic!` sites in `src/node.rs` are flagged debt pending migration to `WorkflowError` — do not add new ones.)

## See also
- `README.md` — full design · `docs/PRODUCT_CANON.md` §10/§12.2 · Spec 28 (port-driven routing).
