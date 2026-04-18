---
name: nebula-sdk
role: Integration Author SDK (Re-export Façade)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L1-3.5, L1-4.4, L1-7]
related: [nebula-action, nebula-credential, nebula-resource, nebula-schema, nebula-workflow, nebula-plugin, nebula-validator, nebula-core]
---

# nebula-sdk

## Purpose

An integration author writing a Nebula node should not need to know which of the eight-plus
workspace crates to add to `Cargo.toml` — they should import one crate and get the action
traits, schema types, credential model, resource model, workflow builder, and test harness.
Without a façade, every new contributor discovers the dependency graph by trial and error, which
violates the §4.4 north star (focused day, no plumbing). `nebula-sdk` is that façade: a single
crate that re-exports the common integration surface and provides the `prelude`, workflow
builder, and test runtime that cover the canonical use cases.

## Role

*Integration Author SDK (Re-export Façade).* Re-exports the cross-cutting integration surface
— `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`, `nebula-workflow`,
`nebula-plugin`, `nebula-validator` — through a single dependency. Provides `prelude`,
`WorkflowBuilder`, `ActionBuilder`, and a `TestRuntime` / `RunReport` for integration testing.

## Public API

Top-level re-exports (full crates):

- `nebula_action` — action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`,
  `ResourceAction`), `ActionContext`, `ActionResult`, `ActionError`, `ActionMetadata`.
- `nebula_credential` — credential model and accessor trait.
- `nebula_resource` — resource model and lifecycle types.
- `nebula_schema` — `Field`, `Schema`, `FieldValues`, proof-token pipeline.
- `nebula_workflow` — workflow definition types, `DependencyGraph`.
- `nebula_plugin` — `Plugin` trait, `PluginMetadata`, `PluginRegistry`.
- `nebula_validator` — validation traits.
- `nebula_core` — core ID types (`ExecutionId`, `NodeKey`, `WorkflowId`).

Modules provided by this crate:

- `prelude` — one-stop `use nebula_sdk::prelude::*` import for common types and traits.
- `action` — `ActionBuilder` for programmatic action metadata construction.
- `workflow` — `WorkflowBuilder` for programmatic workflow construction.
- `runtime` — `TestRuntime`, `RunReport` — in-process test execution harness.
- `testing` (feature `testing`) — test helpers and fixtures.

Macros:

- `params!` — create `FieldValues` from key-value pairs.
- `json!` — re-export of `serde_json::json!`.
- `workflow!` — declarative workflow definition macro.
- `simple_action!` — convenience macro for simple `ProcessAction` implementations.

SDK-level error:

- `Error` — `Workflow`, `Action`, `Parameter`, `Serialization`, `Other` variants.

## Contract

- **[L1-§3.5]** The SDK surface covers the five integration concepts: Action, Credential,
  Resource, Schema, Plugin. It does not introduce new integration concepts — adding a sixth
  requires canon revision (§0.2).

- **[L1-§4.4]** DX is a first-class contract. Breaking changes to the `prelude` or
  `WorkflowBuilder` API affect all integration authors — treat with the same care as a public
  SDK surface (§7, open source contract).

- **[L1-§7]** Public integration / plugin SDK surface: stability matters; breaking changes
  need explicit announcement and migration guidance, not drive-by commits.

## Non-goals

- Not the engine or runtime — this crate is for writing integrations, not for deploying or
  driving executions. See `nebula-engine` for that.
- Not an expression evaluator — see `nebula-expression`.
- Not a plugin process binary entry point — see `nebula-plugin-sdk` (`run_duplex`).
- Does not re-export `nebula-resilience` directly — resilience pipelines are composed at the
  action call site; authors import `nebula-resilience` explicitly if needed.

## Maturity

See `docs/MATURITY.md` row for `nebula-sdk`.

- API stability: `partial` — `prelude`, `WorkflowBuilder`, `ActionBuilder` are in active use;
  the `testing` module and `TestRuntime` are usable but the harness coverage is still growing.
- `anyhow` is re-exported for convenience despite `CLAUDE.md` preferring `thiserror` in
  library crates — this is a deliberate ergonomics choice for integration authors (scripts and
  one-off nodes) but new first-party integrations should prefer typed errors.
- `simple_action!` macro covers the common case but more complex action shapes (stateful,
  trigger, resource-backed) require direct trait implementation.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5, §4.4, §7, `docs/INTEGRATION_MODEL.md`.
- Glossary: `docs/GLOSSARY.md` §1 (integration model), §3 (action model).
- Siblings: `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`,
  `nebula-workflow`, `nebula-plugin`, `nebula-validator`.
