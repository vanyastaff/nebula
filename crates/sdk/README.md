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
learn the workspace topology. External one-dependency proofs cover typed action authoring,
`ActionMetadataDraft`, `WorkflowBuilder`, credential `TestResult`, and representative derives for
every current procedural-macro family. Manual `Provider` authoring with a consuming terminal hook is
also compile-checked using the SDK plus the general-purpose `async-trait` crate;
other manual/prelude workflows need their own compile-pass proof before
being described as verified.
Without a façade, every new contributor discovers the dependency graph by trial and error, which
violates the §4.4 north star (focused day, no plumbing). `nebula-sdk` is that façade: a single
crate that provides persona-scoped integration contracts, while uncurated workflows are recorded
as SDK gaps rather than hidden direct-dependency recipes.

## Role

*Integration Author SDK (Persona Façade).* Provides curated authoring contracts through
`prelude`, persona modules such as `integration`, `ActionMetadataDraft`, `WorkflowBuilder`, and a
`TestRuntime` / `RunReport` for integration testing. Workspace implementation crates are not
re-exported: crate-boundary refactors must not become integration migrations.

## Public API

Supported entry points are `nebula_sdk::prelude`, the `action` / `workflow` authoring modules,
`integration`, `runtime`, and feature-gated `testing`. The sole supported credential-test path is
`nebula_sdk::integration::credential::{TestFailureCode, TestResult}`. A hidden `__private` module
exists only so exported declarative and procedural macros can resolve their implementation dependencies; it is
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

The SDK re-exports the current Action, Credential, Plugin, Resource, Schema, and Validator derive
families. Their generated paths prefer a directly declared leaf crate (including a renamed
dependency) and otherwise resolve through `nebula_sdk::__private`. The external
`derive_consumer` fixture compiles representative derives with `nebula-sdk` as its only
dependency. `__private` remains unstable implementation plumbing and must not be used directly by
integrations. It is nevertheless part of Rust's reachable public namespace, so its explicit
allowlist contains no manager, erased factory, admission request, or structural slot-identity
types. A topology-bearing `Resource` derive returns an opaque SDK-owned token from
`<Name>Factory::into_contribution()`; runtime factory erasure and registration authority stay
behind that token.

### Typed action and schema authoring

`AuthoredValue` accepts any JSON root. Use `AuthoredValue::from_data(json)` for
data ingress: template-looking strings and `{"$expr": ...}` objects remain data.
Use `AuthoredValue::from_template_json(json)` only when the author explicitly opts
into template shorthand. The SDK macro makes the same choice mandatory:

```rust
use nebula_sdk::prelude::*;

#[derive(Deserialize, Schema)]
#[serde(crate = "nebula_sdk::serde")]
struct Greeting {
    name: String,
}

let authored = params! { data; "name" => "{{ literal text }}" }?;
let schema = schema_of::<Greeting>()?;
assert_eq!(authored.to_json(), json!({"name": "{{ literal text }}"}));
assert!(schema.find(&field_key!("name")).is_some());
# Ok::<(), Box<dyn std::error::Error>>(())
```

`params! { template; "name" => "{{ $input.name }}" }` opts into expression
authoring and also returns `Result<AuthoredValue, ValidationError>`. For mixed
intent, start with an authored object and insert
`AuthoredValue::Expression(Expression::new(source))`
at explicit locations. A declared expression-permitting field is required, and
an ancestor's `ExpressionMode::Forbidden` denies expressions throughout that subtree.
Shorthand and `Expression::new` use `ProgramSyntax::Auto`, so a lone envelope
retains its JSON type. Use `Expression::template(source)` for always-string text,
or `Expression::with_syntax(source, ProgramSyntax::Expression)` for raw grammar.
The curated `ProgramSyntax` is independent of expression permission.

**Breaking migration:** erased `ActionInput` / `PreparedActionInput`, `ValueTree`,
`ValidValues`, `ResolvedValue`, and `ResolvedValues` are runtime proof types and are no
longer exported by the SDK. Action authors declare `Action::Input` and `Action::Output`;
the runtime validates and decodes input once before calling typed `execute`. Credential
authors likewise receive `&Self::Properties` in `Credential::resolve`. The authored
`AuthoredValue` stage remains public because `params!` produces it explicitly.

Authored-value serde uses a version-2 `{ "version": 2, "data": ..., "expressions": [...] }`
envelope with required `{path, syntax, source}` entries. Syntax is exactly `auto`,
`expression`, or `template`; serialization preserves it even from compiled values.
It is not a raw JSON ingress
API. `AuthoredValue::from_data(json)` ingests data only and does not enable template shorthand.

The curated `RootShape` distinguishes Any, Scalar, Record, and Union. `()` and
unit structs have a null scalar schema; empty braced structs remain records.
Primitive schemas retain their JSON kind and numeric bounds. Author one root with
`params! { data; 42_u8 }` or `params! { data; () }`; `params! { data; }` still
authors `{}`. `params! { template; "{{ 42 }}" }` explicitly authors a program,
but scalar schemas do not admit expressions at their root.

Resource configurations with fields supply their own schema, for example
`#[derive(Clone, Schema, ResourceConfig)]` with `#[config(schema = external)]`.
Standalone `ResourceConfig` derives supply a schema only for unit/null and genuinely
empty braced records; they never publish an empty contract for a populated struct.

`HasSchema::schema()` and `schema_of::<T>()` remain fallible authoring contracts.
`Action::metadata`, `Credential::metadata`, and `Provider::metadata` return schema-free
drafts. Runtime factories derive schemas from the associated companion types and own
the only admission into terminal metadata. `ActionMetadata`, `CredentialMetadata`,
`ResourceMetadata`, `BaseMetadata`, and admission errors are therefore not supported
SDK persona types.

### Catalog construction migration

Use the three concrete drafts from `integration::{action, credential, resource}`
or `prelude`. Each has `new(key, MetadataName, description)` and fallible
`try_new(key, name, description)`. Credential constructors no longer take an auth
pattern; the registry derives it from the associated scheme. Resource authors
implement `Provider::metadata` explicitly; `from_key` is removed. Action macros
preserve full SemVer, including prerelease and build metadata.

`integration` and `prelude` curate `CatalogCategoryKey`, `CatalogLink`,
`CatalogLinkRelation`, `CatalogLinkTarget`, `DocumentationOrigin`, `CatalogReference`,
`VersionReq`, `RemovalDate`, `RemovalMilestone`, `RemovalSchedule`, and the checked
`DeprecationNotice` builders/getters. `CatalogValueError`, `MetadataError`, and
`MetadataField` are nameable diagnostics. These are authoring values, not admitted
proofs or factory authority. Categories, tags, and links are canonicalized at
admission; documentation URLs project Overview links. Typed replacement references
do not register dependencies or authorize execution.

Migrate the removed lifecycle and diagnostic forms explicitly:

| Before | After |
|--------|-------|
| `.reason(text)` | `.with_reason(text)` |
| `.replacement(text)` | `.with_replacement(CatalogReference::action(action_key!("example.next")))`; select the matching credential, resource, or plugin reference constructor. |
| `.sunset(text)` | `.with_removal(RemovalSchedule::OnDate(RemovalDate::new(2028, 2, 29)?))`, or a typed `AtVersion` / `Milestone` schedule. |
| Public notice fields / struct literals | `DeprecationNotice::new(since)` plus builders; read with `since()`, `removal()`, `replacement()`, and `reason()`. |
| `ManifestError::InvalidKey(_)` | `ManifestError::InvalidKey`; invalid input is no longer retained in the error. |
| Numeric casts of `MetadataError` | Match typed variants (including their `MetadataField`), or use the existing `Classify` diagnostic contract where available; enum discriminants are not diagnostic codes. |

Host catalog storage must migrate to nested wire-v2 `base` records; old flat or
unversioned evidence is rejected and must be replaced from fresh definitions.
See [catalog migration and bounds](../../docs/INTEGRATION_MODEL.md#catalog-construction-and-wire-migration).

### Resource authoring and the SDK

Resource authoring types, traits, and derives are in the prelude and the explicit
`nebula_sdk::integration::resource` persona:

| Surface | What you use |
|--------|----------------|
| **Prelude** | `nebula_sdk::prelude::*` re-exports the author surface: derives `Resource` / `ResourceConfig` / `ClassifyError`; traits `Provider`, `ResourceConfig`, `HasCredentialSlots`, `PoolProvider`, `ResidentProvider`, `BoundedProvider`; topologies `Pooled`, `Resident`, `Bounded` with `PoolConfig` / `ResidentConfig` / `BoundedMode`; and `ResourceMetadataDraft`, `ResourceContext`, `ResourceGuard`, `ReleaseOutcome`, `ResourceKey`, `resource_key!`, `ScopeLevel`, `SlotCell`, `TopologyTag`, `ReloadOutcome`, `Error`, `ErrorKind`, `no_credential_slots!`. See `prelude.rs` for a runnable pooled-resource example. |
| **Derives** | `Resource` and `ResourceConfig` are covered by the SDK-only derive compile contract. Manual `Provider` authoring, including a consuming `destroy` over a non-Clone instance using `TeardownCx` and `TeardownReason`, is separately compile-checked through the prelude plus the general-purpose `async-trait` crate. |

**Release migration:** `ResourceGuard::release()` now returns
`Result<ReleaseOutcome, Error>` instead of `Result<(), Error>`. Match
`ReleaseOutcome::Completed`, `ReleaseOutcome::Deferred`, and `_` because the
enum is non-exhaustive. A deferred release has consumed the guard and
transferred ownership to bounded, best-effort queue cleanup; never retry it,
and do not interpret it as proof that the provider hook will run.

**Custom topology authoring:** `nebula_sdk::integration::resource` curates the open
`Topology` contract, built-in provider hooks, terminal context, and store vocabulary.
The external SDK-only fixture (plus general-purpose `async-trait`) compiles a custom
topology with a non-Clone provider and instance, explicit credential hook, admission,
load, maintenance, and retained-lease signatures. This proves authoring reachability;
runtime cleanup and cancellation guarantees are tested by the resource crate.

Custom topologies are trusted in-process adapters. Framework-supplied `InstanceStore`
and `RetainedStore` access is registration-local lifecycle capability, including the
inherent mutation methods of `InstanceStore`. It does not grant global registry or
tenant authority. Authors must preserve ownership and credential fences: the type
system cannot prevent an adapter from hiding aliases or dropping extracted entries.
`Manager`, `Registry`, `ReleaseQueue`, `ResourceFactory`, registration requests,
and structural slot identities are excluded from this persona and checked by
negative external import probes. This applies equally to `__private`: hidden
documentation is not an access boundary.

**Terminal hook migration:** Remove `Provider::shutdown(&Instance)` implementations;
that hook was never framework-driven. Move asynchronous flush, drain, stop, close,
and worker-join work into `destroy(Instance, TeardownCx)`. Import `TeardownCx` and
`TeardownReason` from `nebula_sdk::prelude`. This source-breaking cleanup keeps
the SDK and its internal packages on their existing lockstep version.

`destroy` consumes the final owned instance even on error and is never retried by
the framework. Its default only runs synchronous Drop. Overrides must tolerate
the manager already being cancelled and need drop fallback for tasks/handles if
the future is abandoned at any await. Bound graceful work with
`tokio::time::timeout_at(cx.deadline.into(), …)`; the framework captures its
deadline independently, so changing the context's public fields cannot extend
it. Cooperative timeouts cannot preempt blocking polls or Drop, and neither
async cleanup nor Drop promises release after a process crash.

**Not in the prelude:** the engine-owned lifecycle (`Manager::register` / `acquire_*`, `Registry`, dispatch, rotation fan-out). Authors implement `Provider`; the engine drives it. Missing authoring contracts are SDK gaps, not permission to reach through to implementation crates.

Note that prelude `Error` is `nebula_resource::Error` (the resource error type); `thiserror::Error` is a derive macro in a separate namespace, so both coexist under the glob. The SDK's own error is exported as `SdkError`.

Modules provided by this crate:

- `prelude` — one-stop `use nebula_sdk::prelude::*` import for common types and traits.
- `integration` — curated, persona-scoped contracts for integration authors.
- `action` — typed action and `ActionMetadataDraft` authoring contracts.
- `workflow` — `WorkflowBuilder` for programmatic workflow construction.
- `runtime` — `TestRuntime`, `RunReport` — in-process test execution harness.
- `testing` (feature `testing`) — test helpers and fixtures.

Macros:

- `params!` — author an `AuthoredValue` root or named-parameter object with explicit `data;` or `template;` intent.
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
  The supported plugin-author contract is curated through this SDK.
- Does not currently curate `nebula-resilience`; a missing author contract is an SDK gap rather
  than a supported direct dependency on that technical crate.

## Maturity

See `docs/MATURITY.md` row for `nebula-sdk`.

- API stability: `partial` — `prelude`, `WorkflowBuilder`, and draft metadata are in active use;
  the `testing` module and `TestRuntime` are usable but the harness coverage is still growing.
- General-purpose serde/JSON/`thiserror` conveniences are re-exported; `anyhow` is not part of the
  SDK surface. First-party libraries use typed errors.
- `simple_action!` macro covers the common case but more complex action shapes (stateful,
  trigger, resource-backed) require direct trait implementation.
- External one-dependency proofs cover typed action drafts, `simple_action!`, `WorkflowBuilder`, credential
  `TestResult`, and representative Action/Credential/Plugin/Resource/Schema/Validator derives.
- The external public-perimeter fixture also compile-checks manual `Provider` terminal
  authoring with `nebula-sdk` as its only Nebula dependency plus `async-trait`.
  Runtime teardown guarantees are covered by the resource crate's tests.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5, §4.4, §7, `docs/INTEGRATION_MODEL.md`.
- Siblings: `nebula-action`, `nebula-credential`, `nebula-resource`, `nebula-schema`,
  `nebula-workflow`, `nebula-plugin`, `nebula-validator`.
