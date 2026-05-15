# 0052 — Credential management runtime crate (`nebula-credential-runtime`)

- **Status:** accepted (2026-05-15)
- **Tags:** credential, runtime, layer-boundary, breaking, supersession, m11
- **Narrowly supersedes:** the facade-ownership slice of
  `C:/Users/vanya/RustroverProjects/docs/adr/0030-engine-owns-credential-orchestration.md`

## Context

The credential contract crate (`nebula-credential`) is internally
complete, but the subsystem has no owner for the *management bounded
context*: a populated `CredentialTypeRegistry`, the
validate→encrypt→CAS-store pipeline, lifecycle dispatch by capability,
store-or-external state resolution, and the observability seam. The API
service layer is 12 `503` stubs precisely because that owner does not
exist. ADR-0030 placed the low-level resolver/RefreshCoordinator/lease
mechanism in `nebula-engine` but did not create a management facade;
folding one into `nebula-engine` would conflate the workflow-execution
engine with credential management and leave the security invariants
(layered store, non-optional observer, tenant scoping) enforced by
discipline rather than by a crate boundary.

`deny.toml` facts: only `nebula-api`/`nebula-cli` may depend on both
`nebula-engine` and `nebula-storage` (both Exec). The facade needs both,
so it must be Exec tier — it cannot be a Business-tier crate.

## Decision

Introduce `nebula-credential-runtime` (Exec tier). It is the sole owner
of the credential management facade; its only public entry is
`CredentialService`, with all invariant-bearing composition crate-private
so the secure construction path is the only path. It depends on
`nebula-engine` (Exec sibling, curated) for the existing low-level
resolver/RefreshCoordinator/lease mechanism — acyclic: `nebula-engine`
does **not** depend on the runtime.

This narrowly supersedes ADR-0030's facade slice only: ADR-0030's
mechanism (resolver, RefreshCoordinator, claim repo) stays in
`nebula-engine`. ADR-0041 (durable refresh claim repo) and ADR-0051
(external provider redesign) are untouched; ADR-0051's deferred Phase-D
non-goal ("wire `ExternalProvider::resolve` into resolution") is
*fulfilled* by the runtime's `StateSource`, not worked around.

## Consequences

- `deny.toml` gains a `nebula-credential-runtime` wrapper entry; the
  allowlist widens to `{ nebula-api, nebula-cli }` when the facade lands.
- `nebula-api` depends on `nebula-credential-runtime` for credential
  management (its `nebula-engine` dep remains for workflow execution).
- Breaking: the API credential service surface changes from stubs to a
  real facade-backed implementation.

## Deferred ideal (recorded so it is not lost)

Full extraction — relocating the engine's resolver / lease / rotation /
RefreshCoordinator / claim-repo into the runtime crate so
`nebula-engine` is de-godded — is the cleaner long-term decomposition.
It is **deferred**: relocating the chaos-tested ADR-0041 claim-repo
against a "finalize to stable" goal is unacceptable risk for this
effort. Revisit as a dedicated migration ADR.

## ADR-0028 cross-crate canon-audit checklist

The runtime implementation (Plans 2–3) must satisfy all eight ADR-0028
invariants; each is gated by a test or compile-fail probe:

1. §12.5 encryption-at-rest preserved — runtime composes the layered
   store (`Scope(Audit(Cache(Encryption(raw))))`); compile-fail probe:
   raw backend unusable without layers.
2. §13.2 refresh/rotation seam integrity — no silent strand; explicit
   `ReauthRequired`.
3. Stored-state vs projected-auth-material split — responses built from
   `CredentialSnapshot` only.
4. No discard-and-log audit — audit sink refusal → `StoreError::AuditFailure`.
5. §4.5 honesty gating — MATURITY/status vocabulary respected.
6. Compat re-exports — no shims; importers updated directly.
7. No new storage behaviour without canon — runtime adds no new
   `CredentialStore` semantics, only composes existing layers.
8. Cross-crate compat cycle — acyclic (engine ⇏ runtime) verified by
   `cargo deny check bans`.
