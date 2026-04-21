---
id: 0033
title: integration-credentials-plane-b
status: accepted
date: 2026-04-21
supersedes: []
superseded_by: []
tags: [credential, integration-model, auth-scheme, oauth2, canon-3.5, canon-12.5, canon-13.2, plane-b]
related:
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0032-credential-store-canonical-home.md
  - docs/INTEGRATION_MODEL.md
  - docs/PRODUCT_CANON.md#35-integration-model
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#132-rotation-refresh-seam
  - crates/credential/README.md
  - crates/credential/src/scheme/auth.rs
linear: []
---

# 0033. Integration credentials (Plane B) — canonical model

## Context

Nebula distinguishes **how a workflow authenticates to external systems**
(integrations: SaaS APIs, databases, webhooks) from **how a human or service
authenticates to Nebula itself** (platform login, SSO, LDAP bind to the
control plane). Confusing the two produces the “everything is OAuth” trap
and duplicates controllers the way monolithic products mix user auth with
integration secrets.

[Canon §3.5](../PRODUCT_CANON.md#35-integration-model) already states that the
engine owns the stored-state vs projected auth-material split for
integrations. [ADR-0028](./0028-cross-crate-credential-invariants.md) through
[ADR-0032](./0032-credential-store-canonical-home.md) split persistence,
orchestration, and OAuth HTTP across crates — but they do **not** name the
**conceptual** boundary this work serves.

This ADR **names and freezes** that boundary as **Plane B — integration
credentials**, so new protocols and crates (including a future
**`nebula-auth`** for Plane A) can attach without collapsing layers.

## Decision

### 1. Definition — Plane B (integration credentials)

**Plane B** is the set of **typed integration accounts** that workflows use to
call **external** systems. Each account is an instance of a type implementing
[`Credential`](../../crates/credential/src/contract/credential.rs): it has
encrypted **state**, optional **pending** interactive state, projects to a
consumer-facing [`AuthScheme`](../../crates/credential/src/scheme/auth.rs),
and may support refresh, test, or revocation per associated consts.

Plane B is **not** “OAuth only.” OAuth2 is **one** family of integration
auth; others include static secrets, password pairs, key material, assertions,
and future plugin-defined schemes — see `AuthPattern` in
[`scheme/auth.rs`](../../crates/credential/src/scheme/auth.rs).

### 2. Explicit non-scope — Plane A (platform authentication)

**Plane A** — who may call Nebula’s API/UI (sessions, API keys for the host,
future SSO/SAML/OIDC/LDAP **to Nebula**). A dedicated **`nebula-auth`** (or
equivalent) crate is **expected** for Plane A; it **must not** implement
`Credential` for operator login, and **must not** store integration secrets in
the same conceptual bucket without an explicit, reviewed bridge.

Plane A ADRs are **out of scope** for this document; Plane B ADRs must not
require Plane A design choices.

### 3. Separation of concerns — acquisition vs material vs persistence

Three orthogonal axes:

| Axis | Question | Lives in |
| --- | --- | --- |
| **Acquisition mechanism** | How did the secret first enter the system? (form, OAuth redirect, file upload, pasted token, …) | HTTP/UI adapters in **`nebula-api`** (per protocol family), callers into engine/credential pipeline |
| **Auth material shape** | What does action code receive? | **`AuthScheme`** + **`AuthPattern`** — classification for resources and tooling |
| **Persistence** | What is encrypted at rest? | **`CredentialState`** via **`CredentialStore`** in **`nebula-storage`** (per ADR-0029/0032) |

OAuth2 is **one acquisition path** (authorization code, client credentials,
device code) that produces **`OAuth2Token`** material. API key is **another**
acquisition path producing **`SecretToken`**. The same `AuthPattern` may be
filled by different acquisition paths (e.g. manual token vs OAuth) only if the
team **explicitly** designs two `Credential` types or a single type with
clear, validated branches — avoid silent ambiguity in `FieldValues`.

### 4. Crate responsibilities for Plane B

Aligned with [ADR-0028](./0028-cross-crate-credential-invariants.md) — this
table states **intent**; migration debt is tracked in MATURITY and the
credential cleanup spec.

| Crate | Owns for Plane B |
| --- | --- |
| **`nebula-credential`** | `Credential` trait, `State` / `Pending`, `AuthScheme`, `CredentialStore` **trait**, errors, encryption primitives, **pure** helpers (e.g. auth URL construction without HTTP where feasible). **No** dependency on `nebula-engine` or `nebula-api`. |
| **`nebula-engine`** | Runtime orchestration: resolve/continue/refresh/test dispatch, `CredentialResolver`, `RefreshCoordinator`, execution-time policy. **No** HTTP server. |
| **`nebula-storage`** | Concrete `CredentialStore` / `PendingStateStore` impls, rows, layers, migrations. **No** business rules for token exchange. |
| **`nebula-api`** | HTTP **adapters** for integration setup flows that need transport (redirect/callback, future upload endpoints). Delegates to engine/credential; does **not** redefine `Credential` semantics. |

### 5. Adding a new integration auth mechanism

1. **Classify** — map to an existing [`AuthPattern`](../../crates/credential/src/scheme/auth.rs) or extend `AuthScheme` / pattern only after UI/tooling impact is reviewed (`#[non_exhaustive]` on `AuthPattern`).
2. **Implement `Credential`** — new `struct` + `impl Credential` **or** extend a **family** type (e.g. new OAuth2 grant) if the protocol is truly the same family.
3. **Set capability consts** — `INTERACTIVE`, `REFRESHABLE`, etc.; default is `false` per trait docs.
4. **Wire acquisition** — if browser/redirect/file upload is required, add **narrow** routes under `nebula-api` (feature-gated if needed); keep **one** HTTP client strategy per token endpoint family (avoid a fourth copy of the same POST — see ADR-0031 / cleanup spec).
5. **Persist** — only through `CredentialStore`; split **config** vs **runtime** secrets where git/export could wipe tokens (peer lesson: `docs/research/n8n-credential-pain-points.md`).

### 6. OAuth2 in Plane B (normative)

`OAuth2Credential` is **the** built-in integration type for **OAuth 2.0 client**
flows to **external** authorization servers. It is **not** a stand-in for
platform SSO. Multiple grants may live **inside** one `Credential` type when
they share state shape and token endpoint semantics; otherwise prefer a
separate type.

HTTP to the token endpoint remains subject to [ADR-0031](./0031-api-owns-oauth-flow.md)
and the ongoing relocation of transport out of the contract crate (optional
`oauth2-http` feature in `nebula-credential` is an incremental migration tool,
not the end state).

## Consequences

### Positive

- Clear vocabulary for reviews: “is this Plane A or B?”
- Plugin and product authors can add **LDAP/SAML to Nebula** (Plane A) without
  overloading `Credential` types meant for Slack/AWS (Plane B).
- Acquisition vs `AuthPattern` vs persistence is explicit — reduces DRY
  violations in HTTP handlers.

### Negative / accepted

- Requires discipline in **`nebula-api`** module naming (`auth` vs
  `integration` / `credentials`) so Plane A and B do not share a single
  `credential` module forever.
- Existing code may mix concerns until refactors land; this ADR is the **target**
  shape, not a claim that every file already conforms.

### Follow-up (non-blocking)

- Reference this ADR from `crates/credential/README.md` and
  `docs/INTEGRATION_MODEL.md` (cross-links).
- When **`nebula-auth`** is introduced, add a **Plane A** ADR that references
  this one under “explicit non-overlap.”

## Alternatives considered

1. **Single “AuthService” crate for all protocols** — Rejected: collapses
   Plane A and B and repeats n8n-scale coupling; violates bounded contexts in
   `architectural-fit` skill.
2. **One `Credential` type with stringly `provider` field** — Rejected:
   breaks typed `project()`, `State`, and schema proofs.
3. **Put all integration HTTP in `nebula-credential` forever** — Rejected:
   already superseded by ADR-0029–0031 and optional `oauth2-http` split.

## Seam / verification

- **Trait seam:** `Credential` + `CredentialStore` in `nebula-credential` —
  integration **meaning** is enforced only here.
- **Canon:** [§3.5 Integration model](../PRODUCT_CANON.md#35-integration-model),
  [§12.5 Secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth),
  [§13.2 Rotation / refresh](../PRODUCT_CANON.md#132-rotation-refresh-seam).
- **Tests:** existing credential + engine + API tests for resolve/refresh;
  new acquisition paths require **at least** unit tests on the `Credential`
  impl and, when HTTP is involved, contract tests on the adapter boundary.
