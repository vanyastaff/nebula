# nebula-sdk — current design

| Field | Value |
|---|---|
| Status | Partial, curated public façade |
| Reviewed | 2026-07-22 |
| Layer | API / supported Rust surface |
| Canon | Root `AGENTS.md` API-boundary invariant; product canon §3.5/§4.4/§7 |

## Boundary

`nebula-sdk` is the sole supported and branded Rust dependency for integration authors. Its public
API is organized by persona; it is not a mirror of the workspace crate graph. Internal product
crates may be lockstep implementation dependencies of the published SDK, but paths such as
`nebula_sdk::nebula_credential` or `nebula_sdk::nebula_resource` are intentionally absent.

The hidden `__private` module exists only so exported `macro_rules!` expansions can resolve
implementation paths in downstream crates. It is not documented, versioned as a persona, or valid
for direct use.

## Supported surface

- `prelude` — curated authoring types and traits.
- Metadata authoring is draft-only. `ActionMetadataDraft`, `CredentialMetadataDraft`, and
  `ResourceMetadataDraft` share curated author vocabulary (`MetadataName`, `MetadataVersion`,
  `Icon`, `DeprecationNotice`) without exposing `MetadataDraft` or any admitted/recorded form.
  Runtime factories alone bind schemas and create terminal metadata.
- `integration` — narrow integration contracts; credential integrations expose
  `ResolveResult`, `StaticResolveResult`, `TestFailureCode`, and `TestResult` here.
- `action::ActionMetadataDraft` and `workflow::WorkflowBuilder` — programmatic authoring.
- `runtime::{TestRuntime, RunReport}` and feature-gated `testing` — integration test support.
- `params!`, `workflow!`, `simple_action!`, and `json!` — SDK-owned macro entry points.
- `Error` / `Result` plus selected general-purpose ecosystem re-exports.

Storage repositories, owner selectors, authorization and input proofs, terminal metadata,
credential runtime constructors, action adapters, engine managers, erased resource factories,
registration requests, structural slot identities, admin writers, and unscoped resolvers are
deliberately outside the surface. `#[doc(hidden)]` affects documentation only, so the macro ABI
is held to the same authority boundary as every documented persona.

## Schema foundation boundary

The prelude names the authored value stage plus schema construction and diagnostic
types. Compiled, validated, resolved, and erased-input proofs remain runtime-owned.
It does not glob-export the schema implementation crate. `params!` requires `data;`
or `template;` intent and returns fallible authored construction. A mixed-intent
object can be built from data with explicit `AuthoredValue::Expression` nodes.
`ProgramSyntax` is curated separately from expression permission: shorthand stays
AUTO, while `Expression::template` retains always-string intent through authored
wire and compiled evaluation.

`RootShape` is the authoritative Any/Scalar/Record/Union contract. The curated prelude names
its shape descriptors and `ScalarKind`/`ScalarSchema` without leaf-crate globs. Unit types
and unit structs describe null, primitive types retain their exact scalar domains, and
empty braced records still describe objects. `params! { data; () }` authors null;
`params! { data; }` authors an empty object. A single value can also opt into template
authoring, but scalar schemas do not grant root-expression admission.
Populated resource configs provide `HasSchema` through a `Schema` derive or manual
implementation and opt out of auto-generation with `#[config(schema = external)]`.
Only unit and genuinely empty braced configs may use the standalone derive's schema.

`schema_of` remains fallible because authors can supply invalid schema definitions.
Typed `Action::Input`, `Action::Output`, and `Credential::Properties` are the public
boundary. SDK test runtime methods accept ordinary JSON and drive the same private
factory admission path as production without exposing its proof tokens.

An exact declaration must permit an authored expression. Permission is not inherited
from a parent, while an ancestor denial forbids expressions in its entire subtree.
Data ingress never turns expression-looking JSON into a program.

## Dependency direction

The façade imports lower product layers and projects only author-facing contracts. Product crates
never depend upward on the SDK. Durable runtime commands do not travel through the SDK; the future
`client` façade submits versioned transport requests and the future `embedded` façade submits typed
runtime commands through curated builders.

## Contract proofs

`tests/public_perimeter_external_contract.rs` builds a real external fixture whose manifest has
exactly one Nebula dependency: `nebula-sdk`. The positive binary exercises the currently supported
manual authoring subset: typed `Action::Input`/`Output`, `ActionMetadataDraft`, `simple_action!`,
`WorkflowBuilder`, and credential `TestResult`. Independent negative binaries prove that internal
authority constructors, owner selectors, raw writers, proof values, terminal metadata, action
adapters, runtime constructors, and unscoped resolvers are not reachable, and assert the intended
compiler diagnostic rather than accepting an unrelated failure.
This fixture does not prove procedural-derive authoring. The independent
`derive_external_contract` fixture covers SDK-only and renamed-leaf derives,
  including typed credential properties and draft metadata. `foundation_contract` executes the
  authored-value, schema-shape, typed action, credential, and macro laws.

## Invariants

- Missing authoring functionality is an SDK gap, not permission to add a direct implementation-
  crate dependency.
- Public credential failures are payload-free typed codes; provider text and secrets never enter
  an SDK result.
- Exported declarative macro implementation paths must stay under `$crate`. Procedural derives must
  resolve an SDK-owned path when the downstream manifest contains only `nebula-sdk`; no macro may
  require authors to name Nebula implementation crates.
- SDK-only resource derives produce an opaque `ResourceContribution` through
  `<Name>Factory::into_contribution()`. The hidden bridge may construct that token from typed
  author contracts, but it exposes no manager, registration request, slot identity, terminal
  metadata, or erased runtime factory trait.
- Breaking changes require release notes and a migration path. Intentional removal of broad crate
  re-exports is a breaking perimeter correction.
- Every type named by a supported author trait or SDK-owned public signature must be reachable
  through an SDK path. Macro-only implementation types stay under `__private`.

## Known gaps

- The dedicated `client` and `embedded` persona façades are not shipped yet.
- Derive expansion is covered for representative Action, Credential, Plugin, Resource,
  Schema, and Validator inputs. New generated paths still need SDK-only and renamed
  consumer proofs; direct leaf dependencies are not a supported workaround for gaps.
- The prelude remains broad; each further contraction needs matching compile-pass and compile-fail
  fixtures for the affected author workflow.
- `derive` is currently an empty feature and should either gate a real surface or be removed in an
  intentional release change.

These gaps are stated as incomplete capability, not as compatibility aliases or hidden support
promises.
