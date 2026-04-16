# nebula-workflow

Workflow definition, DAG graph, and validation for the Nebula workflow engine.

**Layer:** Core
**Canon:** §10 (golden path — activation runs `validate_workflow`), §12.2 (workflow validation as a shift-left contract)

## Status

**Overall:** `implemented` — definition, builder, DAG, and multi-error validator are the authoritative authoring surface.

**Works today:**

- `WorkflowDefinition` + `NodeDefinition` + `Connection` / `EdgeCondition` type system
- `WorkflowBuilder` fluent construction API
- `DependencyGraph` (`petgraph`-backed) — topological sort, per-level batching (feeds `ExecutionPlan`)
- `validate_workflow` — comprehensive multi-error validator
- 7 unit tests covering builder, validator, and graph behaviour

**Known gaps / not in scope:**

- **Activation wiring** — canon §10 requires `validate_workflow` to run at activation, not only at a standalone `/validate` endpoint. Enforcement of that rule lives in `nebula-api` handlers, not here. If an API handler flips workflow state without calling this validator, that is an API bug, not a workflow-crate bug.
- **Persistence** — JSON round-trip and storage belong to `nebula-storage` + `nebula-api`. This crate only defines the types `serde` serializes.
- **Expression evaluation** — dynamic field values are evaluated by `nebula-expression`; this crate only carries the unresolved `ParamValue`.
- **Integration tests** — 0 end-to-end tests in `tests/`; DAG edge cases covered by unit tests only.

## Architecture notes

- **Minimal dependency surface.** Only `nebula-core` and `nebula-error` — no business-layer or execution-layer imports. This is correct for a Core-layer crate.
- **Panics** (4 `panic!` sites) — used as builder-invariant guards. The fluent `WorkflowBuilder` should ideally surface all violations via `WorkflowError`; any remaining panics are documentation debt.
- **No obvious SRP/DRY violations.** Nine modules split cleanly: definition (data), graph (structure), validate (contract enforcement), builder (authoring), state (runtime progress).
- **No dead code or compat shims.**

## Scope

This crate defines workflows as **directed acyclic graphs** of action nodes connected by conditional edges. It owns the definition types, the dependency graph, the fluent builder, and the multi-error validator used at activation time.

## What this crate provides

| Type / function | Role |
| --- | --- |
| `WorkflowDefinition` | Top-level workflow + supporting config types. |
| `NodeDefinition` | Individual step (action key, params, rate limit, …). |
| `ParamValue` | Typed value variant for node parameters. |
| `Connection`, `EdgeCondition` | Edges between nodes, with conditional routing. |
| `ErrorMatcher`, `ResultMatcher` | Pattern matching for conditional edges on action outcomes. |
| `DependencyGraph` | `petgraph` wrapper with topological sorting and per-level computation (feeds `ExecutionPlan`). |
| `WorkflowBuilder` | Fluent, validated construction API. |
| `validate_workflow` | Comprehensive multi-error validator. **Canon §10 requires this to run at activation**, not only on a standalone `/validate` endpoint. |
| `NodeState` | Tracks execution progress for a node definition. |
| `WorkflowError`, `RateLimit` | Supporting types. |

## Non-goals

- **Not** the execution state — see `nebula-execution`.
- **Not** the storage layer — persisted workflow JSON and activation are handled by `nebula-api` + `nebula-storage`.
- **Not** an expression evaluator — see `nebula-expression` for dynamic fields.

## Where the contract lives

- Source: `src/lib.rs`, `src/definition.rs`, `src/validate.rs`, `src/graph.rs`
- Canon: `docs/PRODUCT_CANON.md` §10, §12.2
- Schema compatibility is a public surface — see `docs/UPGRADE_COMPAT.md`.

## See also

- `nebula-execution` — runtime state that consumes the DAG via `ExecutionPlan`
- `nebula-engine` — orchestrator that validates + schedules workflows
- `nebula-validator` — declarative rule engine used inside validation
