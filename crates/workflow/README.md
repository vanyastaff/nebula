---
name: nebula-workflow
role: Workflow Definition + DAG + Validation (shift-left contract)
status: stable
last-reviewed: 2026-04-17
canon-invariants: [L2-10, L2-12.2]
related: [nebula-execution, nebula-expression, nebula-schema, nebula-validator, nebula-core, nebula-error]
---

# nebula-workflow

## Purpose

A workflow engine needs a durable, round-trippable representation of what to execute. Without
a shared definition type, every layer — API, storage, engine, validation — would parse and
re-interpret workflow JSON independently, producing silent mismatches between what an operator
saves and what the engine runs. `nebula-workflow` is that shared representation: a
`WorkflowDefinition` that round-trips through serde, a `DependencyGraph` that computes the
execution order, a `WorkflowBuilder` that assembles definitions ergonomically, and
`validate_workflow` that enforces the invariants the engine relies on. The validator must run
at activation — that is the shift-left contract that canon §10 requires.

## Role

**Workflow Definition + DAG + Validation.** The Core-layer crate that all higher layers
(API, engine, storage) import for the shared definition type and the activation-time validator.
Only `nebula-core` and `nebula-error` are imported; no upward dependencies.

Pattern: *Make illegal states unrepresentable* (DMMF) — `validate_workflow` returns a
`Result` with all errors collected, not a side-effect on a mutable definition. The caller
cannot activate a workflow without explicitly handling the result.

## Public API

- `WorkflowDefinition` — top-level workflow struct; carries `WorkflowConfig`,
  `Vec<NodeDefinition>`, `Vec<Connection>`, `UiMetadata`.
- `NodeDefinition` — individual step: `ActionKey`, params (`HashMap<String, ParamValue>`),
  `RateLimit`, `RetryConfig`, position.
- `ParamValue` — typed value variant for node parameters (static literal or expression string).
- `Connection` — directed wire `(from_node, to_node, from_port, to_port)`. Edges carry no
  conditions or matchers; conditional and error routing live in explicit `ControlAction`
  nodes (the trait lives in `nebula_action::control`; canonical implementations — `If`,
  `Switch`, `Router`, `Filter`, `NoOp`, `Stop`, `Fail` — ship downstream in a reference /
  plugin crate, not this workspace) per Spec 28 §2.2. Failed nodes activate only edges
  whose `from_port == "error"` — authors wire that port into whichever `ControlAction`
  fits. See `src/connection.rs` module doc for the full activation contract. The pre-Spec-28
  `EdgeCondition` / `ResultMatcher` / `ErrorMatcher` trio was removed when this design landed.
- `DependencyGraph` — `petgraph`-backed wrapper: topological sort, per-level batching (feeds
  `ExecutionPlan` in `nebula-execution`).
- `WorkflowBuilder` — fluent, validated construction API.
- `validate_workflow` — comprehensive multi-error validator. **Canon §10 requires this to run
  at activation**, not only at a standalone `/validate` endpoint. Seam: `crates/workflow/src/validate.rs`.
- `NodeState` — tracks execution progress per node definition.
- `WorkflowError` — structured error type for builder and validator failures.
- `Version`, `CURRENT_SCHEMA_VERSION` — workflow schema version management.

## Contract

- **[L2-§10]** `validate_workflow` is the canonical activation gate. An API handler that
  enables a workflow without calling this function violates canon §10 golden path step 2.
  Seam: `crates/workflow/src/validate.rs` — `validate_workflow`. The enforcement of the call
  lives in `nebula-api` activation handlers; this crate owns the function itself.
- **[L2-§12.2]** Workflow validation is a shift-left contract: misconfiguration (dangling
  connections, duplicate node IDs, cycles, invalid action keys) must be rejected at
  activation with structured RFC 9457 errors, not silently deferred to runtime dispatch
  failures. Test: `crates/workflow/src/validate.rs` tests, builder tests.
- **JSON round-trip** — `WorkflowDefinition` must survive a `serde_json` round-trip
  without information loss. Workflow definition schema compatibility is a public surface
  (see `docs/UPGRADE_COMPAT.md`). Seam: `crates/workflow/src/definition.rs`.

## Non-goals

- Not the execution state machine — see `nebula-execution` (`ExecutionStatus`,
  `ExecutionState`, `ExecutionPlan`).
- Not the storage layer — JSON persistence and activation state live in `nebula-storage`
  and `nebula-api`.
- Not an expression evaluator — `ParamValue` carries unresolved expression strings;
  resolution is `nebula-expression`'s job.
- Not a DAG executor — `DependencyGraph` computes topological order; execution scheduling
  is `nebula-engine`'s job.

## Maturity

See `docs/MATURITY.md` row for `nebula-workflow`.

- API stability: `stable` — definition types, builder, DAG, and validator are the
  authoritative authoring surface in active use by API and engine layers.
- **Activation wiring** is not enforced by this crate — if an API handler skips
  `validate_workflow`, that is an API-layer bug, not a workflow-crate gap.
- 4 `panic!` sites remain as builder-invariant guards; these are documentation debt
  (should surface via `WorkflowError`).
- Integration tests: 0 in `tests/`; DAG edge cases covered by unit tests only.

## Related

- Canon: `docs/PRODUCT_CANON.md` §10 (golden path — activation runs `validate_workflow`),
  §12.2 (validation as shift-left contract).
- Upgrade compat: `docs/UPGRADE_COMPAT.md` — workflow schema is a public compatibility surface.
- Siblings: `nebula-execution` (consumes `DependencyGraph` via `ExecutionPlan`),
  `nebula-engine` (orchestrates; calls `validate_workflow` at activation),
  `nebula-expression` (resolves `ParamValue` expressions at runtime).

## Appendix

### Architecture notes

- Minimal dependency surface: only `nebula-core` and `nebula-error`. No imports from
  engine, runtime, storage, or API layers — the `CLAUDE.md` Core-layer direction is respected.
- Nine modules split cleanly: `definition` (data), `graph` (structure), `validate` (contract
  enforcement), `builder` (authoring DX), `state` (runtime progress tracking).
- No dead code or compatibility shims.
