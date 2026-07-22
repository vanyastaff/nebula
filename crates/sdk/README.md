---
name: nebula-sdk
role: Integration Author SDK (Persona Façade)
status: partial
last-reviewed: 2026-04-22
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
crate that provides persona-scoped integration contracts, the `prelude`, workflow builder, and
test runtime that cover the canonical use cases.

## Role

*Integration Author SDK (Persona Façade).* Provides curated authoring contracts through
`prelude`, persona modules such as `integration`, `WorkflowBuilder`, `ActionBuilder`, and a
`TestRuntime` / `RunReport` for integration testing. Broad workspace-crate re-exports remain
temporarily for compatibility; they expose internal topology and are not stable SDK personas.

## Public API

The sole supported credential-test path is
`nebula_sdk::integration::credential::{TestFailureCode, TestResult}`.

Temporary top-level compatibility re-exports (internal topology, not stable SDK personas):

- `nebula_action` — action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`,
  `ResourceAction`), `ActionContext`, `ActionResult`, `ActionError`, `ActionMetadata`.
- `nebula_credential` — credential model and accessor trait.
- `nebula_resource` — resource model and lifecycle types.
- `nebula_schema` — `Field`, `Schema`, `FieldValues`, proof-token pipeline.
- `nebula_workflow` — workflow definition types, `DependencyGraph`.
- `nebula_plugin` — `Plugin` trait, `PluginManifest`, `PluginRegistry`.
- `nebula_validator` — validation traits.
- `nebula_core` — core ID types (`ExecutionId`, `NodeKey`, `WorkflowId`).

### Credential, OAuth, and the SDK

Integration authors consume credential contracts through curated SDK personas:

| Surface | What you use |
|--------|----------------|
| **Curated integration contract** | `nebula_sdk::integration::credential::{TestFailureCode, TestResult}` for provider credential-test outcomes. This is the supported SDK path for this contract. |
| **Prelude** | `nebula_sdk::prelude::*` re-exports the common credential and OAuth2 types used in actions (`Credential`, `OAuth2Credential`, `OAuth2Token`, `CredentialContext`, `CredentialSnapshot`, …) — see `prelude.rs`. |
| **Temporary compatibility re-export** | `nebula_sdk::nebula_credential` exposes internal crate topology. It remains during active development for compatibility, but is unsupported for new integrations and is not a stable SDK persona. |

**Not in the SDK:** HTTP token exchange/refresh against a provider, storage encryption, and engine `CredentialResolver` — those are product/runtime concerns. If a contract needed by integration authors is absent from a curated SDK persona, treat that as an SDK API gap rather than depending on the temporary broad re-export.

**Migration:** Provider tests import `TestFailureCode` and `TestResult` only from the curated integration path above and construct `TestResult::Failed { code }`; the removed `reason` field is not accepted. K4 may remove broad runtime or topology leaks while preserving this curated persona path. For other credential/OAuth moves, follow the SDK release notes and this README rather than internal crate topology.

### Resource authoring and the SDK

Resource authoring types and traits are in the prelude; derive macros still need
their target crates on the dependency graph:

| Surface | What you use |
|--------|----------------|
| **Full crate** | `nebula_sdk::nebula_resource` (same as depending on `nebula-resource` directly). Engine-integrator types live here: `Manager`, `Registry`, `ReleaseQueue`, `credential_fanout`, metrics, recovery gates. |
| **Prelude** | `nebula_sdk::prelude::*` re-exports the author surface: derives `Resource` / `ResourceConfig` / `ClassifyError`; traits `Provider`, `ResourceConfig`, `HasCredentialSlots`, `PoolProvider`, `ResidentProvider`, `BoundedProvider`; topologies `Pooled`, `Resident`, `Bounded` with `PoolConfig` / `ResidentConfig` / `BoundedMode`; and `AcquireOptions`, `RegistrationSpec`, `ResourceContext`, `ResourceGuard`, `ResourceMetadata`, `ResourceKey`, `resource_key!`, `ScopeLevel`, `SlotIdentity`, `SlotCell`, `TopologyTag`, `ReloadOutcome`, `Error`, `ErrorKind`, `no_credential_slots!`. See `prelude.rs` for a runnable pooled-resource example. |
| **Direct deps for derives** | `#[derive(Resource)]` expands to `::nebula_resource::…` / `::nebula_core::…` paths — add `nebula-resource`, `nebula-core`, and `async-trait` as direct `Cargo.toml` dependencies (match the versions `nebula-sdk` pins). `impl Provider` needs `#[async_trait::async_trait]`. |

**Not in the prelude:** the engine-owned lifecycle (`Manager::register` / `acquire_*`, `Registry`, `ReloadOutcome` dispatch, rotation fan-out). Authors implement `Provider`; the engine drives it. If you outgrow the prelude list, import from `nebula_sdk::nebula_resource` without adding another workspace dependency.

Note that prelude `Error` is `nebula_resource::Error` (the resource error type); `thiserror::Error` is a derive macro in a separate namespace, so both coexist under the glob. The SDK's own error is exported as `SdkError`.

Modules provided by this crate:

- `prelude` — one-stop `use nebula_sdk::prelude::*` import for common types and traits.
- `integration` — curated, persona-scoped contracts for integration authors.
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
- Plugins register in-process through `nebula-plugin` — there is no separate plugin process binary (ADR-0091).
- Does not re-export `nebula-resilience` directly — resilience pipelines are composed at the
  action call site; authors import `nebula-resilience` explicitly if needed.

## Maturity

See `docs/MATURITY.md` row for `nebula-sdk`.

- API stability: `partial` — `prelude`, `WorkflowBuilder`, `ActionBuilder` are in active use;
  the `testing` module and `TestRuntime` are usable but the harness coverage is still growing.
- `anyhow` is re-exported for convenience despite `AGENTS.md` preferring `thiserror` in
  library crates — this is a deliberate ergonomics choice for integration authors (scripts and
  one-off nodes) but new first-party integrations should prefer typed errors.
- `simple_action!` macro covers the common case but more complex action shapes (stateful,
  trigger, resource-backed) require direct trait implementation.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5, §4.4, §7, `docs/INTEGRATION_MODEL.md`.
- Siblings: `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`,
  `nebula-workflow`, `nebula-plugin`, `nebula-validator`.
