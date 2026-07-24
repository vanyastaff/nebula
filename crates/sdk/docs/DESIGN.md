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
- `integration` — narrow integration contracts; credential tests currently expose
  `TestFailureCode` and `TestResult` here.
- `action::ActionBuilder` and `workflow::WorkflowBuilder` — programmatic authoring.
- `runtime::{TestRuntime, RunReport}` and feature-gated `testing` — integration test support.
- `params!`, `workflow!`, `simple_action!`, and `json!` — SDK-owned macro entry points.
- `Error` / `Result` plus selected general-purpose ecosystem re-exports.

Storage repositories, owner selectors, authorization proofs, credential runtime constructors,
engine managers, admin writers, and unscoped resolvers are deliberately outside the surface.

## Dependency direction

The façade imports lower product layers and projects only author-facing contracts. Product crates
never depend upward on the SDK. Durable runtime commands do not travel through the SDK; the future
`client` façade submits versioned transport requests and the future `embedded` façade submits typed
runtime commands through curated builders.

## Contract proofs

`tests/public_perimeter_external_contract.rs` builds a real external fixture whose manifest has
exactly one Nebula dependency: `nebula-sdk`. The positive binary exercises the currently supported
manual/builder subset: `ActionBuilder`, `WorkflowBuilder`, and credential `TestResult`. Independent
negative binaries prove that internal authority constructors, owner
selectors, raw writers, admin repositories, runtime constructors, and unscoped resolvers are not
reachable, and assert the intended compiler diagnostic rather than accepting an unrelated failure.
This fixture does not prove procedural-derive authoring.

## Invariants

- Missing authoring functionality is an SDK gap, not permission to add a direct implementation-
  crate dependency.
- Public credential failures are payload-free typed codes; provider text and secrets never enter
  an SDK result.
- Exported declarative macro implementation paths must stay under `$crate`. Procedural derives must
  resolve an SDK-owned path when the downstream manifest contains only `nebula-sdk`; no macro may
  require authors to name Nebula implementation crates.
- Breaking changes require release notes and a migration path. Intentional removal of broad crate
  re-exports is a breaking perimeter correction.

## Known gaps

- The dedicated `client` and `embedded` persona façades are not shipped yet.
- Action, credential, plugin, resource, schema, and validator procedural derives still emit or
  fall back to implementation-crate paths. Derive-based authoring is therefore not yet inside the
  strict one-Nebula-dependency perimeter. Manual/builder authoring is curated; direct leaf-crate
  dependencies are not a supported workaround.
- The prelude is still broad and will need persona-focused contraction with compile-pass and
  compile-fail fixtures for each supported workflow.
- `derive` is currently an empty feature and should either gate a real surface or be removed in an
  intentional release change.

These gaps are stated as incomplete capability, not as compatibility aliases or hidden support
promises.
