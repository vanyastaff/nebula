---
name: nebula-sdk
role: Integration Author SDK (Persona Façade)
status: partial
last-reviewed: 2026-07-22
canon-invariants: [L1-3.5, L1-4.4, L1-7]
related: [nebula-action, nebula-credential, nebula-resource, nebula-schema, nebula-workflow, nebula-plugin, nebula-validator, nebula-core]
---

# nebula-sdk

## Purpose

The product contract is that an integration author should depend on one Nebula crate rather than
learn the workspace topology. Today the external one-dependency proof covers the narrow
manual/builder subset of `ActionBuilder`, `WorkflowBuilder`, and credential `TestResult`; other
manual/prelude workflows need their own compile-pass proof before being described as verified.
Without a façade, every new contributor discovers the dependency graph by trial and error, which
violates the §4.4 north star (focused day, no plumbing). `nebula-sdk` is that façade: a single
crate that provides persona-scoped integration contracts, while uncurated workflows are recorded
as SDK gaps rather than hidden direct-dependency recipes.

## Role

*Integration Author SDK (Persona Façade).* Provides curated authoring contracts through
`prelude`, persona modules such as `integration`, `WorkflowBuilder`, `ActionBuilder`, and a
`TestRuntime` / `RunReport` for integration testing. Workspace implementation crates are not
re-exported: crate-boundary refactors must not become integration migrations.

## Public API

Supported entry points are `nebula_sdk::prelude`, the `action` / `workflow` builders,
`integration`, `runtime`, and feature-gated `testing`. The sole supported credential-test path is
`nebula_sdk::integration::credential::{TestFailureCode, TestResult}`. A hidden `__private` module
exists only so exported declarative macros can resolve their implementation dependencies; it is
not a compatibility namespace or a supported persona.

### Credential, OAuth, and the SDK

Integration authors consume credential contracts through curated SDK personas:

| Surface | What you use |
|--------|----------------|
| **Curated integration contract** | `nebula_sdk::integration::credential::{TestFailureCode, TestResult}` for provider credential-test outcomes. This is the supported SDK path for this contract. |
| **Prelude** | `nebula_sdk::prelude::*` re-exports the common credential and OAuth2 types used in actions (`Credential`, `OAuth2Credential`, `OAuth2Token`, `CredentialContext`, `CredentialSnapshot`, …) — see `prelude.rs`. |

**Not in the SDK:** HTTP token exchange/refresh against a provider, storage encryption, and engine `CredentialResolver` — those are product/runtime concerns. If a contract needed by integration authors is absent from a curated SDK persona, treat that as an SDK API gap rather than depending directly on an implementation crate.

**Migration:** Provider tests import `TestFailureCode` and `TestResult` only from the curated integration path above and construct `TestResult::Failed { code }`; the removed `reason` field is not accepted. Old `nebula_sdk::nebula_*` paths are intentionally gone. If a needed contract has no curated path, open an SDK gap instead of adding a direct implementation-crate dependency.

### Procedural derive status

The SDK re-exports derive names, but the generated code for all current Nebula procedural derive
families still names implementation crates directly. Therefore derive-based authoring is **not yet**
inside the strict one-Nebula-dependency perimeter:

- Action derives emit `nebula_action`, `nebula_core`, and `nebula_workflow` paths;
- Credential derives emit `nebula_credential`/`nebula_core` paths;
- Plugin derives emit `nebula_plugin` paths;
- Resource derives emit `nebula_resource`, `nebula_core`, `nebula_credential`, and
  `nebula_schema` paths;
- Schema derives fall back to `nebula_schema` when only the SDK is a direct dependency; and
- Validator derives emit `nebula_validator` paths.

Direct dependencies on those leaves may make generated code compile, but they are not a supported
SDK workaround. K4 must make the procedural derives SDK-resolvable and add external one-dependency
compile-pass fixtures for every supported derive workflow.

### Resource authoring and the SDK

Resource authoring types and traits are in the prelude; derive macros still need
their target crates on the dependency graph:

| Surface | What you use |
|--------|----------------|
| **Prelude** | `nebula_sdk::prelude::*` re-exports the author surface: derives `Resource` / `ResourceConfig` / `ClassifyError`; traits `Provider`, `ResourceConfig`, `HasCredentialSlots`, `PoolProvider`, `ResidentProvider`, `BoundedProvider`; topologies `Pooled`, `Resident`, `Bounded` with `PoolConfig` / `ResidentConfig` / `BoundedMode`; and `AcquireOptions`, `RegistrationSpec`, `ResourceContext`, `ResourceGuard`, `ResourceMetadata`, `ResourceKey`, `resource_key!`, `ScopeLevel`, `SlotIdentity`, `SlotCell`, `TopologyTag`, `ReloadOutcome`, `Error`, `ErrorKind`, `no_credential_slots!`. See `prelude.rs` for a runnable pooled-resource example. |
| **Current derive limitation** | Resource derives are part of the general procedural-derive gap above. Manual `Provider` authoring is the currently curated path and uses the prelude plus the general-purpose `async-trait` crate. |

**Not in the prelude:** the engine-owned lifecycle (`Manager::register` / `acquire_*`, `Registry`, dispatch, rotation fan-out). Authors implement `Provider`; the engine drives it. Missing authoring contracts are SDK gaps, not permission to reach through to implementation crates.

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
- `simple_action!` — convenience macro for simple `StatelessAction` implementations.

SDK-level error:

- `Error` — `Workflow`, `Action`, `Parameter`, `Serialization`, `Other` variants.

## Contract

- **[L1-§3.5]** The SDK's canonical target covers the five integration concepts: Action,
  Credential, Resource, Schema, Plugin. Current workflow maturity is documented explicitly; naming
  a concept does not claim every derive/client/embedded path is already shipped. Adding a sixth
  concept requires canon revision (§0.2).

- **[L1-§4.4]** DX is a first-class contract. Breaking changes to the `prelude` or
  `WorkflowBuilder` API affect all integration authors — treat with the same care as a public
  SDK surface (§7, open source contract).

- **[L1-§7]** Public integration / plugin SDK surface: stability matters; breaking changes
  need explicit announcement and migration guidance, not drive-by commits.

## Non-goals

- Not the engine or runtime — this crate is for writing integrations, not for deploying or
  driving executions. See `nebula-engine` for that.
- Not an expression evaluator — see `nebula-expression`.
- Plugins are trusted in-process adapters — there is no separate plugin process binary (ADR-0091).
  The supported plugin-author contract must be curated through this SDK; the current derive path is
  listed above as incomplete.
- Does not currently curate `nebula-resilience`; a missing author contract is an SDK gap rather
  than a supported direct dependency on that technical crate.

## Maturity

See `docs/MATURITY.md` row for `nebula-sdk`.

- API stability: `partial` — `prelude`, `WorkflowBuilder`, `ActionBuilder` are in active use;
  the `testing` module and `TestRuntime` are usable but the harness coverage is still growing.
- General-purpose serde/JSON/`thiserror` conveniences are re-exported; `anyhow` is not part of the
  SDK surface. First-party libraries use typed errors.
- `simple_action!` macro covers the common case but more complex action shapes (stateful,
  trigger, resource-backed) require direct trait implementation.
- The external one-dependency proof currently covers `ActionBuilder`, `WorkflowBuilder`, and
  credential `TestResult`. Action/Credential/Plugin/Resource/Schema/Validator procedural derives
  remain unverified and leaf-path-dependent.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5, §4.4, §7, `docs/INTEGRATION_MODEL.md`.
- Siblings: `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`,
  `nebula-workflow`, `nebula-plugin`, `nebula-validator`.
