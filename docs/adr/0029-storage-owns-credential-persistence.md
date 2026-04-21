---
id: 0029
title: storage-owns-credential-persistence
status: accepted
date: 2026-04-20
supersedes: [0023]
superseded_by: []
tags: [credential, storage, security, canon-12.5, persistence, key-custody]
related:
  - docs/adr/0023-keyprovider-trait.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0031-api-owns-oauth-flow.md
  - docs/adr/0021-crate-publication-policy.md
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/STYLE.md#6-secret-handling
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
linear: []
---

# 0029. `nebula-storage` owns credential persistence

## Context

[ADR-0028](./0028-cross-crate-credential-invariants.md) establishes the
umbrella of cross-crate credential invariants for the architecture
cleanup. This ADR codifies the first of three migrations: **all
persistence-related credential types move from `nebula-credential` to
`nebula-storage/src/credential/`**. The credential crate becomes a pure
contract crate (trait + DTOs + §12.5 primitives); the storage crate
owns the persistence impl.

The concrete motivation is §1 of the design spec
([`docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md`](../superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md)):
`nebula-credential` currently owns
`CredentialStore` trait + `InMemoryStore` impl + `EncryptionLayer` +
`CacheLayer` + `AuditLayer` + `ScopeLayer` + `KeyProvider` + pending
store + backup store. Every one of those is persistence work. That SRP
violation breaks symmetry with the sibling `nebula-storage` crate
(which already hosts `repos/` for non-credential stores) and leaves
`nebula-credential` with a conflated responsibility set.

[ADR-0023](./0023-keyprovider-trait.md) named `KeyProvider` as a
public contract in `nebula-credential`. That ADR's trait shape,
invariants, rotation semantics, and three impl contracts (`Env`,
`File`, `Static`) remain correct; only the crate that owns them
changes. This ADR supersedes ADR-0023 in the **location** of
`KeyProvider` and `EncryptionLayer`, not in their design.

The canon context that binds this migration:

- [§12.5 — Secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth)
  — AES-256-GCM authenticated encryption at rest, Argon2id KDF, AAD
  binding. Invariant 1 of ADR-0028.
- [§14 — Anti-patterns](../PRODUCT_CANON.md#14-anti-patterns) —
  no discard-and-log on audit. Invariant 4 of ADR-0028.
- [STYLE.md §6 — Secret handling](../STYLE.md#6-secret-handling) —
  `Zeroize` / `ZeroizeOnDrop`, `Zeroizing<T>` intermediates, no
  plaintext in error strings, log-redaction test helper.

## Decision

### 1. Supersede relationship with ADR-0023

**This ADR supersedes ADR-0023 in the _location_ of the `KeyProvider`
trait. The trait shape, `ProviderError` variants, rotation-safety
fingerprint scheme, three in-tree impls (`EnvKeyProvider`,
`FileKeyProvider`, `StaticKeyProvider`), and every other contract
from ADR-0023 are preserved bit-for-bit — only the crate that owns
them changes.**

- Trait definition moves from
  `crates/credential/src/layer/key_provider.rs` to
  `crates/storage/src/credential/key_provider.rs`.
- `EncryptionLayer` moves from
  `crates/credential/src/layer/encryption.rs` to
  `crates/storage/src/credential/layer/encryption.rs`.
- AAD binding at the existing `encryption.rs:146-211` (credential id is
  AAD; record-swapping is rejected; AAD-less records are rejected) is
  ported without modification. ADR-0028 invariant 1 applies.
- `with_legacy_keys` rotation path and the `""` → `"default"` migration
  story land verbatim in the new location.
- SHA-256-prefix `version()` scheme (`env:<fp>`, `file:<name>:<fp>`)
  remains observable SemVer surface of the storage crate, inheriting
  from ADR-0023 §5.

### 2. Canonical file paths in `nebula-storage`

The target shape under `crates/storage/src/credential/` (see spec §3):

```
crates/storage/src/credential/
├── mod.rs
├── store.rs              # CredentialStore trait + StoredCredential + PutMode + StoreError
├── memory.rs             # InMemoryStore (feature: credential-in-memory)
├── key_provider.rs       # KeyProvider + EnvKeyProvider + FileKeyProvider + StaticKeyProvider
├── pending.rs            # pending state repo (encrypted at rest; see §4)
├── backup.rs             # rotation backup repo
└── layer/
    ├── mod.rs
    ├── encryption.rs     # EncryptionLayer (§12.5 AAD preserved)
    ├── cache.rs          # CacheLayer (moka)
    ├── audit.rs          # AuditSink trait + in-line durable default impl
    └── scope.rs          # ScopeLayer
```

The following types move from `nebula-credential` to `nebula-storage`
at the listed destinations:

| From `nebula-credential/src/` | To `nebula-storage/src/credential/` |
|---|---|
| `store.rs` (`CredentialStore`, `StoredCredential`, `PutMode`, `StoreError`) | `store.rs` |
| `store_memory.rs` (`InMemoryStore`) | `memory.rs` |
| `layer/encryption.rs` (`EncryptionLayer`) | `layer/encryption.rs` |
| `layer/key_provider.rs` (`KeyProvider` + impls) | `key_provider.rs` |
| `layer/cache.rs` (`CacheLayer`, `moka`-backed) | `layer/cache.rs` |
| `layer/audit.rs` (`AuditSink`, `AuditLayer`) | `layer/audit.rs` |
| `layer/scope.rs` (`ScopeLayer`) | `layer/scope.rs` |
| `pending_store.rs`, `pending_store_memory.rs` | `pending.rs` |
| `rotation/backup.rs` | `backup.rs` |

### 3. `nebula-credential` reexports

`nebula-credential::lib.rs` keeps a stable re-export for `CredentialStore`:

```rust
pub use nebula_storage::credential::CredentialStore;
```

This re-export is **permanent**, not transitional. Consumers importing
`nebula_credential::CredentialStore` do not need to chase the path
change. ADR-0028 invariant 6 applies: re-exports may expose trait +
error + DTO shapes, but not storage impl details (no `InMemoryStore`,
no `CacheLayer`, no backend-specific hints).

### 4. Pending store invariants

The pending store (`crates/storage/src/credential/pending.rs`) holds
interactive-flow state (OAuth2 PKCE `code_verifier`, `state` token,
CSRF binding) between the authorization-request step and the callback
step. Because `code_verifier` is a secret, the pending store carries
security invariants equivalent to the encrypted credential store, with
additional TTL and single-use semantics.

The following invariants are **non-negotiable** and enforced by CI:

1. **Encrypted at rest.** `code_verifier` and any adjacent secret
   fields are wrapped by `EncryptionLayer` on write; the pending repo
   never persists plaintext. AAD binding applies (credential id +
   pending token as AAD per the canonical AAD scheme).
2. **TTL ≤ 10 minutes.** Pending entries auto-expire 10 minutes after
   creation. An entry past its TTL is treated as absent
   (`PendingError::Expired`); no error-message side channel
   distinguishes "never existed" from "expired."
3. **Single-use.** The callback consume step issues a transactional
   `get_then_delete` against the repo; there is no read-without-delete
   API exposed to the OAuth controller. A replay attempt returns
   `PendingError::NotFound` (indistinguishable from expiry).
4. **Request-session binding.** Each pending entry carries the
   originating API request session id; the callback consume step
   rejects (`PendingError::SessionMismatch`) if the callback session
   differs. Defense against cross-session CSRF is in ADR-0031; this
   store's job is to surface a safe error if the binding is violated.
5. **API returns `SecretString`, not `String`.** Read path returns
   `PendingRecord { code_verifier: SecretString, state: SecretString, … }`.
   Raw `String` is not part of the public API of the pending repo.
   ADR-0028 invariant 7 applies.
6. **Zeroize-on-drop on read.** `PendingRecord` derives
   `ZeroizeOnDrop`; the read path's intermediate buffers use
   `Zeroizing<Vec<u8>>`. Scope exit scrubs the decrypted bytes per
   STYLE.md §6.

### 5. Audit sink — in-line durable

`AuditSink` trait + default impl (`storage/src/repos/audit.rs` — already
exists) backs `AuditLayer`. The invariant is **fail-closed**: if the
audit write errors, the credential operation errors. No "log and
continue" path. ADR-0028 invariant 4 is the normative source; this
section declares the storage-side enforcement point.

`AuditEvent` fields (redaction-safe by construction, per STYLE.md §6
and spec §8):

```
AuditEvent {
    verb:         "put" | "get" | "delete" | "rotate" | "refresh",
    credential_id,
    outcome:      Ok | Denied | Error,
    started_at,
    latency_ms,
}
```

**No plaintext value, no ciphertext, no key_id hash.** `AuditEvent`
`Debug` / `Display` are hand-written (not auto-derived) and fmt only
the whitelisted fields above. Future field additions must be
hand-added to `Debug` — auto-derive would silently leak new fields
through `{:?}`.

### 6. SemVer surface of `nebula-storage`

From the day this ADR's implementation lands, the following become
part of `nebula-storage` public SemVer surface (per ADR-0021):

- `CredentialStore` trait + `StoredCredential` / `PutMode` / `StoreError`.
- `KeyProvider` trait + `ProviderError` (`#[non_exhaustive]`) + three
  in-tree impls. Inherits the SemVer commitments from ADR-0023 §5,
  including the fingerprint scheme on `version()`.
- `EncryptionLayer` constructor signatures (`new`, `with_legacy_keys`).
- `PendingStore` trait + `PendingRecord` + `PendingError`.
- `AuditSink` trait + `AuditEvent` (the redacted shape; new fields are
  additive only).

Breaking any of the above requires a superseding ADR.

### 7. Test coverage migration

The existing `crates/credential/src/layer/encryption.rs` mod tests
(round-trip, AAD enforcement, multi-key lazy rotation, CAS-on-rotate
for issue #282, `""` → `"default"` legacy alias for issue #281) move
to a single file at
`crates/storage/tests/credential_encryption_invariants.rs`.

ADR-0028 invariant 1 declares §12.5 tests are **single source of
truth** — not scattered across crates. If a future change requires a
credential-side invariant test, it lives with the contract; if it
requires a storage-side impl test, it lives with the impl. No
duplication.

Pending-store lifecycle tests
(`crates/credential/tests/units/pending_lifecycle_tests.rs`) move
wholesale to `crates/storage/tests/` and pin invariants 1-6 above.

## Consequences

**Positive.**

- `nebula-credential` becomes a pure contract crate. SRP satisfied.
- `nebula-storage` gains a symmetric `credential/` module next to its
  existing repos; credential persistence is discoverable where persistence
  code lives.
- Operators and library embedders see the composition root more
  clearly: `nebula-storage::credential::CredentialStore` is the impl
  trait, and `KeyProvider` implementations (KMS, Vault, env, file) plug
  into the storage crate alongside backend choice. The "where does
  credential key material come from" question has a single answer.
- `EncryptionLayer` wraps any `CredentialStore` impl (in-memory,
  SQLite-backed future, Postgres-backed future) — the §12.5 guarantee
  is decoupled from the backend.
- Pending-store invariants §4 make the OAuth2 PKCE flow's security
  posture explicit and CI-enforced.
- `KeyProvider`'s next impls (`KmsKeyProvider`, `VaultKeyProvider` —
  see ADR-0023 follow-ups) land next to the trait in `nebula-storage`
  instead of reaching back into `nebula-credential`.

**Negative / accepted costs.**

- Import-path churn for callers constructing `EncryptionLayer`,
  `InMemoryStore`, or a `KeyProvider` impl directly. The call site
  stays the same; only the `use` line changes. Workspace has zero
  external publish consumers today (ADR-0021 §Context), so the cost
  is absorbed before `nebula-credential` has an external audience.
- `nebula-storage` takes on a credential-adjacent dependency surface:
  `moka` for caching, `aes-gcm` + `argon2` via `nebula-credential`'s
  `secrets::crypto` primitives. The alternative — duplicating
  primitives in storage — is worse (two implementations of AAD
  binding drift within weeks).
- Two canonical paths for `CredentialStore` (`nebula_credential::…`
  and `nebula_storage::credential::…`). Documented; ADR-0028 invariant
  6 makes the re-export permanent.
- Test file rename changes blame attribution. Acceptable; the tests
  were always co-invariant with the impl.

**Neutral.**

- ADR-0023's design decisions (sync `current_key()`, two-constructor
  surface, `ProviderError` typed variants, `StaticKeyProvider` behind
  `test-util`) are **unchanged**. Only the crate location moves.
- AAD binding at the former `encryption.rs:146-211` — credential-id is
  AAD, AAD-less records rejected, record-swapping rejected — ports
  without modification.
- `nebula-credential::secrets::crypto` (primitives) is called by
  `nebula-storage::credential::layer::encryption` via sibling
  dependency. Crypto primitives stay in credential; impl stays in
  storage. Clean layer direction.

## Alternatives considered

### A. Keep `CredentialStore` in `nebula-credential`, move only `EncryptionLayer` to storage

**Rejected.** Partial move creates a worse taxonomy: the trait lives in
credential, but implementations (encryption wrap, cache wrap, audit wrap)
live in storage. Consumers constructing the composition root would
import from both crates to build a layered store. The alternative
preserves `nebula-credential`'s monolith profile without eliminating
it. Full move is the simpler result.

### B. Create a new `nebula-credential-store` crate

**Rejected.** A new workspace member for credential persistence would
duplicate the `nebula-storage` crate's purpose. `nebula-storage`
already exists for the same reason (consolidated persistence), and
credential persistence is not categorically different from execution
persistence, control-queue persistence, or trigger-lock persistence.
Model B+ from the design spec §4 alternatives.

### C. Keep `KeyProvider` in `nebula-credential`, move only the encryption layer

**Rejected.** The composition root lives next to the impl. Putting
`EncryptionLayer` in storage but `KeyProvider` in credential forces
every storage-side impl site to depend on credential for key material
and on storage for the layer. The trait binds to the layer's
construction — they belong together.

### D. Single `crates/storage/src/credential.rs` file, no submodule

**Rejected.** After the move, the credential module has store + memory
+ three layer impls + key_provider + pending + backup + tests. One
file would push into thousands of lines. Submodule organization (§2)
follows the existing pattern of `crates/storage/src/repos/` — each
repo gets a module, credential is no exception.

### E. Move `secrets::crypto` primitives to storage alongside the impl

**Rejected. Would break ADR-0028 invariant 1.** `EncryptionLayer`
calls crypto primitives, but the primitives are the canon §12.5
surface that every credential-aware crate may reference (including
api for OAuth PKCE — `crypto::pkce_challenge`). Keeping them in
`nebula-credential` (pure Core-layer sibling) means storage, engine,
and api all import primitives from credential without cycle; moving
primitives to storage would force api and engine to depend on storage
for pure arithmetic. Wrong layer.

## Seam / verification

Files that carry the invariants after the migration (spec §3):

- `crates/storage/src/credential/store.rs` — `CredentialStore` trait,
  `StoredCredential`, `PutMode`, `StoreError`.
- `crates/storage/src/credential/memory.rs` — `InMemoryStore` behind
  `credential-in-memory` feature.
- `crates/storage/src/credential/key_provider.rs` — `KeyProvider`
  trait + `EnvKeyProvider` + `FileKeyProvider` + `StaticKeyProvider`
  (inherits ADR-0023 §3 invariants).
- `crates/storage/src/credential/layer/encryption.rs` —
  `EncryptionLayer`; AAD binding preserved.
- `crates/storage/src/credential/layer/cache.rs` — `CacheLayer` on
  `moka`.
- `crates/storage/src/credential/layer/audit.rs` — `AuditSink` trait,
  `AuditLayer` in-line durable default impl, `AuditEvent` redacted
  shape.
- `crates/storage/src/credential/layer/scope.rs` — `ScopeLayer`.
- `crates/storage/src/credential/pending.rs` — `PendingStore` +
  `PendingRecord` + `PendingError`; invariants 1-6 §4.
- `crates/storage/src/credential/backup.rs` — rotation backup repo.
- `crates/credential/src/lib.rs` — permanent `pub use
  nebula_storage::credential::CredentialStore` (ADR-0028 invariant 6).

Test coverage:

- `crates/storage/tests/credential_encryption_invariants.rs` — AAD
  round-trip, multi-key rotation, CAS-on-rotate, legacy alias (migrated
  from `crates/credential/tests/units/encryption_tests.rs`).
- `crates/storage/tests/credential_audit_durable.rs` — mock
  `AuditSink` failure → `put()` returns `StoreError` (not silent
  success). ADR-0028 invariant 4.
- `crates/storage/tests/credential_pending_lifecycle.rs` — TTL expiry,
  single-use delete, session-binding mismatch, zeroize-on-read.
  Invariants §4.1-§4.6.
- `crates/credential/tests/redaction.rs` (remains — shared helper) —
  adds rows for pending-store operations. ADR-0028 invariant 7.

CI signals:

- **Layer direction in `deny.toml`**: `nebula-storage` may depend on
  `nebula-credential`; reverse direction forbidden. Rule lands in the
  same PR as the move (P6), not follow-up.
- **MSRV 1.95**: all new files respect MSRV per
  [ADR-0019](./0019-msrv-1.95.md).

## Follow-ups

- **Phase P6 implementation PR** (spec §12) — physical move of
  `store.rs`, `store_memory.rs`, `layer/`, `key_provider.rs` into
  `nebula-storage/src/credential/`. Re-export in `nebula-credential`
  lands in the same PR.
- **Phase P7 implementation PR** — physical move of `pending_store*.rs`
  and `rotation/backup.rs` into `nebula-storage/src/credential/`.
- **[ADR-0030](./0030-engine-owns-credential-orchestration.md)** —
  downstream sibling; engine consumes `CredentialStore` from
  `nebula-storage` for resolver and rotation orchestration.
- **[ADR-0031](./0031-api-owns-oauth-flow.md)** — downstream sibling;
  api consumes `CredentialStore` and the pending store for OAuth
  callback flow.
- **`KmsKeyProvider` ADR** (was ADR-0023 follow-up) — lands against
  `nebula-storage::credential::KeyProvider` instead of
  `nebula-credential::KeyProvider`. No other change.
- **`VaultKeyProvider` ADR** — same.
- **Key-rotation runbook** under `docs/` (already a follow-up from
  ADR-0023) — updated path references are the only change.
