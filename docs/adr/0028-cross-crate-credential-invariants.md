---
id: 0028
title: cross-crate-credential-invariants
status: accepted
date: 2026-04-20
supersedes: []
superseded_by: []
tags: [credential, storage, engine, api, security, canon-12.5, canon-13.2, canon-3.5, canon-14, canon-4.5]
related:
  - docs/adr/0023-keyprovider-trait.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0021-crate-publication-policy.md
  - docs/adr/0025-sandbox-broker-rpc-surface.md
  - docs/PRODUCT_CANON.md#35-integration-model
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#132-rotation-refresh-seam
  - docs/PRODUCT_CANON.md#14-anti-patterns
  - docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities
  - docs/STYLE.md#6-secret-handling
  - docs/MATURITY.md
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
linear: []
---

# 0028. Cross-crate credential invariants (umbrella)

## Context

`nebula-credential` today is a ~34-module monolith that mixes several
responsibilities: credential contract (what a `Credential` is), storage work
(`CredentialStore` + layers), runtime orchestration (rotation scheduler,
resolver executor), and HTTP flow (OAuth2 reqwest client). This violates SRP
and breaks symmetry with sibling crates `nebula-action` (thin, contract-only)
and `nebula-resource` (holds orchestration internally, also irregular —
addressed by a parallel follow-up spec).

The [`docs/MATURITY.md`](../MATURITY.md) row
(`frontier / stable / stable / partial / n/a`) reflects this:
`Engine integration: partial (rotation in integration tests)` — because
rotation orchestration currently lives in `nebula-credential` instead of
`nebula-engine`. The crate also pulls unnecessary base deps: `reqwest` (for
OAuth HTTP), `nebula-metrics` + `nebula-telemetry` (observability),
`moka` + `lru` (two caches).

The design spec
[`docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md`](../superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md)
describes redistribution of responsibilities across four existing crates
(`credential` → `credential` / `storage` / `engine` / `api`) without creating
new crates, domain-first submodule grouping in target crates, and a phased
landing sequence.

Three follow-up ADRs codify the specific moves:

- [ADR-0029](./0029-storage-owns-credential-persistence.md) — storage owns
  credential persistence (supersedes ADR-0023 in the location of
  `KeyProvider`).
- [ADR-0030](./0030-engine-owns-credential-orchestration.md) — engine owns
  credential orchestration and token refresh.
- [ADR-0031](./0031-api-owns-oauth-flow.md) — api owns OAuth flow HTTP
  ceremony.

This ADR is the **umbrella**: a single normative anchor that the three
migration ADRs cite as the source of invariants. Without it, each migration
could in isolation drift from canon §12.5 (secrets and auth), §13.2
(rotation/refresh seam), §3.5 (stored-state vs auth-material split), §14
(anti-patterns), or §4.5 (operational honesty). The invariants below are
cross-cutting — they bind all four home crates simultaneously, and no single
migration ADR can restate them without drift.

## Decision

This ADR fixes eight cross-crate invariants that apply to every PR in the
credential cleanup series and to every future change that touches credential
material, persistence, orchestration, or transport.

### 1. §12.5 preservation (encryption at rest)

AES-256-GCM + Argon2id + AAD credential-id binding are preserved
**bit-for-bit** across the migration. Primitives (`encrypt`/`decrypt`
functions, `EncryptionKey`, `EncryptedData`) stay in
`nebula-credential/src/secrets/crypto.rs`. The impl (`EncryptionLayer`)
moves to `nebula-storage/src/credential/layer/encryption.rs`; the layer
calls the primitives. **AAD binding code does not change.** Any PR in the
cleanup series that modifies the AAD wiring without a superseding ADR is
canon-level wrong.

### 2. §13.2 seam integrity (non-stranding refresh)

`RefreshCoordinator` (the thundering-herd prevention primitive) stays in
`nebula-credential/src/refresh.rs`. Orchestration concerns — when to
refresh, grace-period windows, transactional state flip — live in
`nebula-engine/src/credential/rotation/`. The seam is **defined** in
credential; the invariant **enforcer** is engine. A PR that moves
`RefreshCoordinator` to engine without a superseding ADR breaks the seam
contract.

See [ADR-0030 §RefreshCoordinator design note](./0030-engine-owns-credential-orchestration.md)
for why the coordinator is kept concrete (not a trait).

### 3. §3.5 stored-state vs auth-material split (preserved)

`Credential::project()` and the `State` / `Pending` associated types stay
in `nebula-credential`. They are not part of the migration. Auth material
derivation is a credential-contract concern, not a persistence or
orchestration concern; moving `project()` would conflate the two
responsibilities that canon §3.5 deliberately separates.

### 4. §14 no discard-and-log (audit durability)

Audit is **in-line durable** — the audit write happens before the
credential operation returns. If the audit sink fails, the whole operation
fails (fail-closed, per spec §8). The fire-and-forget eventbus path
(`CredentialEvent` on `nebula-eventbus`) is **only** for metrics /
dashboards / alerts fanout; it does not replace the audit write and does
not carry security-critical data.

A PR that log-and-discards an audit failure — "log when audit fails,
continue anyway" — is the exact §14 anti-pattern and is rejected
categorically. See
[`CLAUDE.md §"Quick Win trap catalog"`](../../CLAUDE.md) entry
"Log-and-discard on an outbox consumer."

### 5. §4.5 operational honesty (MATURITY gated on landing)

The `Engine integration` column for `nebula-credential` flips from
`partial` to `stable` only after all phases land and the engine actually
drives rotation orchestration end-to-end. The PR series **does not permit**
flipping the MATURITY row before the corresponding code migration (P6
onwards) lands. For `nebula-api`, OAuth flow is feature-gated behind
`credential-oauth` until the `e2e_oauth2_flow` integration test (spec
§13) is green; `credential-oauth` is not a default feature until then.

Advertising a capability in docs that the code does not deliver is a §11.6
false capability and violates §4.5.

### 6. Cross-crate compat invariants

- **`CredentialStore` re-export is permanent, not transitional.** The
  trait moves to `nebula-storage/src/credential/store.rs`, but
  `nebula-credential::lib.rs` keeps `pub use nebula_storage::CredentialStore;`
  (or equivalent) as a stable DX alias. Consumers depending on
  `nebula_credential::CredentialStore` do not need to rewrite imports
  every three months.
- **Re-exports do not leak storage-internal types.** Cache impl details,
  backend-specific hints, and repo internals stay behind the storage
  crate. The credential re-export surface is **only trait + error + DTO
  shapes** — impl detail is hidden.
- **ADR-0023 `KeyProvider` public API moves to storage.** ADR-0029
  supersedes ADR-0023 in the **location** of the trait. Trait shape,
  invariants, and impl contracts from ADR-0023 are preserved bit-for-bit
  — only the crate that owns them changes.
- **`CredentialRecord`, `CredentialMetadata`, `CredentialKey`,
  `CredentialEvent` stay in `nebula-credential`.** They are contract
  types reachable from storage / engine / api via sibling dependencies.

### 7. Zeroize-on-drop at crate boundaries

Any plaintext secret crossing a crate boundary **must** be wrapped in
`SecretString`, `Zeroizing<T>`, or `CredentialGuard`. The following are
rejected at review:

- `credential → storage` handoff of a raw `String` containing a token.
- `storage → engine` handoff of a `Vec<u8>` of plaintext.
- `engine → api` handoff of a bare `&str` containing a refresh token.

The invariant applies **on all four home crates** (credential, storage,
engine, api). The redaction fuzz test (spec §13, §8 CI gates) enforces it:
a crate boundary without a zeroize container that passes a secret-bearing
field is a CI failure.

See [STYLE.md §6](../STYLE.md#6-secret-handling) for the mandatory
patterns, anti-patterns, and log-redaction test helper.

### 8. Versioning discipline during alpha

During the migration, all four home crates use **workspace path-deps**
and do not bump SemVer. `nebula-credential` is named in
[ADR-0021 §3](./0021-crate-publication-policy.md) as part of the initial
publish set, but the acceptance of that commitment was for the post-
migration shape; mid-migration publishes would require every downstream
consumer to chase pre-release versions per phase. Post-migration, the
publishing decision is taken in a separate ADR against the settled shape.

## Consequences

**Positive.**

- Every invariant above is citable from the code that upholds it:
  §12.5 from `secrets/crypto.rs` + `credential/layer/encryption.rs`,
  §13.2 from `refresh.rs` + `credential/rotation/`, §14 from
  `AuditLayer` + `AuditSink`, §4.5 from `docs/MATURITY.md` + feature
  gates. Auditing "is this invariant alive?" is a grep, not an
  archaeology session.
- Each migration ADR (0029 / 0030 / 0031) can cite this umbrella without
  restating the invariants; drift between the three migration ADRs is
  caught by a review gate against this file.
- Consumers of `nebula-credential` see a stable DX surface through the
  migration: `CredentialStore` remains reachable at
  `nebula_credential::CredentialStore`; zeroize wrappers remain the
  canonical secret-bearing types; §12.5 primitives remain in credential.
- Operators do not see silent capability claims: MATURITY and feature
  gates are tied to actual implementation landing per §4.5.

**Negative / accepted costs.**

- Four ADRs land together as a hard go/no-go checkpoint (plan P5) before
  any physical crate move. If even one migration ADR is blocked, the
  entire P6+ sequence stops. This is the intended posture — better a
  clean intermediate state (P1-P5 cleanup landed, cross-crate moves not
  yet started) than a half-migrated workspace with drift.
- The permanent re-export policy (invariant 6) means two canonical
  import paths exist for `CredentialStore` (`nebula_credential::…` and
  `nebula_storage::…`). Documented in `nebula-credential::lib.rs` doc;
  rustdoc rendering links both, consumers pick either. The alternative
  — forcing a global import rewrite every migration — is worse.
- Zeroize-at-crate-boundaries invariant (7) adds redaction-test rows per
  new cross-crate seam. One test per verb/operation per boundary. Spec
  §8 CI gates quantify the ongoing cost (order of 10 tests total across
  the four home crates after migration completes).

**Neutral.**

- The invariants are cross-cutting, not new. §12.5 was already canon;
  §13.2 was already a seam; §14 was already an anti-pattern. This ADR
  makes each enforceable across four crates simultaneously. Nothing
  about canon changes; what changes is the enforcement surface area.
- The umbrella defers specifics to 0029 / 0030 / 0031. Each satellite
  ADR is shorter because it cites this one instead of restating the
  invariants.
- Does not change `KeyProvider`'s shape or semantics from ADR-0023 —
  only its location (handled in ADR-0029).

## Alternatives considered

### A. Model A — status quo

**Rejected.** `nebula-credential` stays a 34-module monolith. Observable
costs: violates SRP, breaks symmetry with `nebula-action` and (the future
symmetric) `nebula-resource`, blocks `Engine integration` MATURITY
improvement, keeps unnecessary base deps (`reqwest`, `nebula-metrics`,
`nebula-telemetry`, redundant caches). The crate would remain the place
where new credential-adjacent concerns land by default, compounding the
problem.

### B. Model B+ — sister crate for rotation orchestration

**Rejected.** A fifth crate (`nebula-credential-rotation`) would own
rotation orchestration. Observable costs: adds workspace-member count
without reducing responsibility sprawl (we would still have credential +
storage + engine + api layers all touching credentials, plus a fifth crate
that only owns rotation). Engine already owns execution orchestration;
rotation is a lifecycle concern of execution, not an independent domain.
Putting rotation adjacent to `engine/control_consumer.rs` (where cancel
and resume orchestration already live) is the right adjacency.

### C. Full n8n-parity immediate

**Rejected.** A one-shot rewrite matching n8n's credential subsystem
(five controllers, one helper, one refresh service, three repos) in a
single PR. Observable costs: blast radius too large to review; every
canon invariant would have to be re-verified against a ~2000-LOC diff
instead of a ~200-LOC diff per phase. The n8n-parity layout is the
**target**; the phased landing is the **path**. This ADR governs the
path; the spec §12 phases define the sequencing.

### D. No ADR, handle invariants in each migration ADR

**Rejected.** Three migration ADRs each restating the same eight
invariants would drift within weeks — one ADR would tighten a condition
while another would loosen it, and no single canonical statement of the
cross-cutting contract would exist. The umbrella pattern (one ADR names
the invariants, three migration ADRs cite them) is the standard resolution
for cross-cutting constraints.

### E. Inline invariants in PRODUCT_CANON.md directly

**Rejected.** The canon sections cited above (§12.5, §13.2, §3.5, §14,
§4.5) already express the invariants at the level of principle. This
ADR translates those principles into the specific cross-crate shape the
four home crates take after migration. Canon should say "secrets are
encrypted at rest"; ADRs should say "the primitive lives here, the impl
lives there, the AAD binding is bit-for-bit preserved, a PR that touches
the AAD wiring opens a superseding ADR." The division of labor between
canon and ADRs is correct as-is.

## Seam / verification

Files that will carry the invariants (post-migration shape, see spec §2):

- `crates/credential/src/secrets/crypto.rs` — §12.5 primitives
  (`encrypt`, `decrypt`, `EncryptionKey`, `EncryptedData`). Invariant 1.
- `crates/storage/src/credential/layer/encryption.rs` — `EncryptionLayer`
  impl; AAD binding preserved bit-for-bit. Invariant 1.
- `crates/credential/src/refresh.rs` — `RefreshCoordinator` primitive.
  Invariant 2.
- `crates/engine/src/credential/rotation/` — orchestration
  (`scheduler.rs`, `grace_period.rs`, `blue_green.rs`, `transaction.rs`,
  `token_refresh.rs`). Invariant 2, 5.
- `crates/credential/src/contract/` — `Credential` trait + `State` /
  `Pending` associated types. Invariant 3.
- `crates/storage/src/credential/layer/audit.rs` — `AuditSink` trait +
  `AuditLayer` (in-line durable). Invariant 4.
- `crates/credential/src/event.rs` — `CredentialEvent` for eventbus
  fanout (metrics only). Invariant 4.
- `crates/storage/src/credential/pending.rs` — encrypted-at-rest pending
  store. Invariant 7 (zeroize at boundary); see ADR-0029.
- `crates/engine/src/credential/rotation/token_refresh.rs` — reqwest
  client + redaction filter. Invariant 7; see ADR-0030.
- `crates/api/src/credential/` — OAuth controller + flow + state.
  Invariant 7; see ADR-0031.
- `docs/MATURITY.md` — credential row, api row. Invariant 5.

Test coverage:

- `crates/storage/tests/credential_encryption_invariants.rs` — AAD
  round-trip, rotation, CAS-on-rotate, legacy alias (migrated from
  `crates/credential/tests/units/encryption_tests.rs`). Invariant 1.
- `crates/storage/tests/credential_audit_durable.rs` — mock audit sink
  fails → operation returns `StoreError`. Invariant 4.
- `crates/storage/tests/credential_eventbus_fanout.rs` — subscriber
  crash → credential op continues unblocked. Invariant 4.
- `crates/credential/tests/redaction.rs` — extended fuzz: one case per
  credential operation (put / get / rotate / refresh / resolve /
  oauth_exchange). Invariant 7.
- `crates/api/tests/e2e_oauth2_flow.rs` — end-to-end cycle across all
  four crates. Invariant 5 (gate on MATURITY flip).

CI signals that catch regressions:

- **Layer direction** (`deny.toml`): credential does not depend on
  storage / engine / api; storage may depend on credential; engine /
  api may depend on credential + storage. Updates land with the
  corresponding P6 / P8 / P10 PRs, not after — policy-layer audit
  blocked until `deny.toml` reflects the new edge.
- **Feature matrix**: CI runs `--all-features` and `--no-default-features`
  legs for `nebula-api` from P10 onwards; without this `credential-oauth`
  silently bitrots between releases.
- **Redaction fuzz**: every new credential verb / cross-crate boundary
  adds a row to the fuzz test that greps all outputs (audit rows,
  eventbus emissions, tracing spans) for secret substrings.

## Follow-ups

- [ADR-0029](./0029-storage-owns-credential-persistence.md) — storage
  owns credential persistence (supersedes ADR-0023 §KeyProvider
  location). Downstream.
- [ADR-0030](./0030-engine-owns-credential-orchestration.md) — engine
  owns credential orchestration + token refresh. Downstream.
- [ADR-0031](./0031-api-owns-oauth-flow.md) — api owns OAuth flow HTTP
  ceremony. Downstream.
- **Phase P6-P11 implementation PRs** per spec §12. Each PR cites this
  ADR + the relevant migration ADR. MATURITY row flip is P11 only.
- **`nebula-resource` symmetric rework** — follow-up spec restores
  symmetry between `nebula-credential` (post-migration) and
  `nebula-resource`. Out of scope for this ADR set.
- **Post-migration publishing decision** — ADR-0021 §3 named
  `nebula-credential` in the initial publish set; the publishing
  decision is revisited against the settled post-migration shape in
  a dedicated ADR.
