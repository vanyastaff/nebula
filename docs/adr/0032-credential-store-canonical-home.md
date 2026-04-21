---
id: 0032
title: credential-store-trait-stays-in-credential
status: accepted
date: 2026-04-20
supersedes: []
superseded_by: []
amends: [0028, 0029]
tags: [credential, storage, dep-graph, trait-impl-split, canon-alpha]
related:
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md
  - docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md
linear: []
---

# 0032. `CredentialStore` trait stays in `nebula-credential`; only impls move to storage

## Context

[ADR-0028](./0028-cross-crate-credential-invariants.md) invariant 6 +
[ADR-0029](./0029-storage-owns-credential-persistence.md) §1-§2 together
specified:

- `CredentialStore` trait + `StoredCredential` + `PutMode` + `StoreError`
  move from `nebula-credential::store` to
  `nebula-storage::credential::store` (ADR-0029 §2 table).
- `nebula-credential::lib.rs` keeps `pub use nebula_storage::credential::CredentialStore;`
  as a permanent DX alias (ADR-0028 invariant 6).
- ADR-0029 §Consequences (Neutral) notes that `nebula-storage::credential::layer::encryption`
  calls the §12.5 crypto primitives living in
  `nebula-credential::secrets::crypto` **via sibling dependency**.

During P6 implementation of the spec plan
[`docs/superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md`](../superpowers/plans/2026-04-20-credential-cleanup-p6-p11.md),
two interlocking problems surfaced:

### Problem 1 — dep-graph cycle

The combination above creates a **cyclic dep-graph**:

- ADR-0028 invariant 6 → `nebula-credential → nebula-storage`
  (for the re-export).
- ADR-0029 §Consequences Neutral → `nebula-storage → nebula-credential`
  (for §12.5 primitives + contract types `CredentialId`,
  `CredentialRecord`, `SecretString`, `CredentialGuard`).

Cargo rejects cyclic package dependencies unconditionally. Feature
flags do not help — Cargo's topo-sort operates at the build-graph
level, before feature resolution.

### Problem 2 — credential's internal consumers of `CredentialStore`

Even if the cycle were resolved (e.g., drop the re-export), moving the
trait breaks `nebula-credential`'s own internal modules that bind to it:

- `crates/credential/src/resolver.rs` declares
  `pub struct CredentialResolver<S: CredentialStore>` and 40+ test sites
  using `InMemoryStore`. This module is scheduled to move to engine in
  P8, not P6.
- `crates/credential/src/registry.rs`, `executor.rs` — similar generic
  bounds.

If the trait moves in P6 but these consumers stay in credential until
P8, credential cannot compile during the P6 → P8 window. Pulling all of
P8's scope into P6 creates a mega-PR that defeats the phased landing
(spec §12, Mode B pauses between phases).

### Resolutions considered

- **A. Drop the re-export + pull consumers forward.** Move the trait
  per ADR-0029, drop the re-export per amended invariant 6, AND move
  `resolver.rs` + `executor.rs` + `registry.rs` from credential to
  engine in the same PR. Combines P6 + P8 partial into one mega-PR.
  Violates Mode B pause cadence.
- **B. New `nebula-credential-primitives` crate.** Extract §12.5
  primitives + contract types (`CredentialId`, `CredentialRecord`, etc.)
  into a new workspace member that both credential and storage depend
  on. Breaks the cycle. Adds a crate; reopens ADR-0029 to pin the new
  location of primitives.
- **C. Trait stays in credential; only impls move to storage.** Split
  the trait/impl axis: credential owns the trait (`CredentialStore`,
  `StoredCredential`, `PutMode`, `StoreError`); storage owns the impls
  (`InMemoryStore`, `EncryptionLayer`, `CacheLayer`, `AuditLayer`,
  `ScopeLayer`, `KeyProvider`). Dep-graph is one-directional
  `storage → credential`. Credential's internal consumers of the trait
  continue to compile unchanged. ADR-0029 §Alternative A rejected this
  on taxonomy grounds, but without the cycle+migration-sequencing
  constraints now surfaced.

This ADR selects **option C**. It has the smallest implementation
scope, preserves the phased landing, and does not require a new
workspace member. ADR-0029 §Alternative A's rejection reason ("consumers
import from both crates") holds, but is the accepted cost: it is
strictly smaller than the mega-PR of option A or the added-crate cost
of option B.

## Decision

### 1. `CredentialStore` trait + DTOs stay in `nebula-credential`

The following types stay in `nebula-credential/src/store.rs` (their
current location):

- `CredentialStore` trait
- `StoredCredential` struct
- `PutMode` enum
- `StoreError` enum
- Any supporting types required by the trait signature.

`nebula-credential::lib.rs` continues to `pub mod store;` and re-export
these types flat at the crate root (unchanged from today's state).

### 2. Impls + layers + key_provider move to `nebula-storage`

The following types move from `nebula-credential` to
`nebula-storage::credential`:

| From `nebula-credential/src/` | To `nebula-storage/src/credential/` |
|---|---|
| `store_memory.rs` (`InMemoryStore`) | `memory.rs` |
| `layer/encryption.rs` (`EncryptionLayer`) | `layer/encryption.rs` |
| `layer/cache.rs` (`CacheLayer`) | `layer/cache.rs` |
| `layer/audit.rs` (`AuditLayer`, `AuditSink`, `AuditEvent`) | `layer/audit.rs` |
| `layer/scope.rs` (`ScopeLayer`, `ScopeResolver`) | `layer/scope.rs` |
| `layer/key_provider.rs` (`KeyProvider` + Env/File/Static impls) | `key_provider.rs` |

Storage's `credential` module imports the trait + DTOs from
`nebula-credential` via sibling dep:

```rust
// crates/storage/src/credential/layer/encryption.rs
use nebula_credential::{CredentialStore, StoredCredential, PutMode, StoreError};
use nebula_credential::{CredentialId, CredentialRecord, encrypt_with_aad, decrypt_with_aad, ...};
```

### 3. One-directional dep graph

```
nebula-credential (contract + trait + primitives + DTOs)
        ↑
        |
nebula-storage::credential (impls + layers + key_provider)
        ↑
        |
nebula-engine, nebula-api (consumers)
```

`nebula-credential → nebula-storage` is **forbidden**. Credential's
`Cargo.toml` MUST NOT list `nebula-storage` in `[dependencies]`.

### 4. Consumer imports

Consumers import:

- **Trait + DTOs** (for trait bounds / handling `StoreError`): from
  `nebula_credential::*` (flat re-exports, as today).
- **Impls** (for composition root / constructing a layered store): from
  `nebula_storage::credential::*`.

Example (post-P6):

```rust
use nebula_credential::CredentialStore;               // trait
use nebula_storage::credential::{InMemoryStore, EncryptionLayer, EnvKeyProvider};

let inner = InMemoryStore::new();
let keyed = EncryptionLayer::new(inner, EnvKeyProvider::from_env("KEY")?);
// `keyed: impl CredentialStore`
```

### 5. Amendments to prior ADRs

#### ADR-0028 invariant 6

The original invariant 6 (permanent re-export of `CredentialStore`
from credential) is **superseded** by this ADR. Amended form:

> **Canonical home per type split on trait/impl axis.** Trait + DTOs
> (`CredentialStore`, `StoredCredential`, `PutMode`, `StoreError`) live
> in `nebula_credential::*`. Impls (`InMemoryStore`, layers, KeyProvider)
> live in `nebula_storage::credential::*`. Consumers import from the
> canonical home for each. Dep-graph is strictly `storage → credential`.
> Credential does **not** re-export from storage.

Other invariants (1-5, 7, 8) are unaffected.

#### ADR-0029 §1-§2

The original §1 (supersede relationship) and §2 (canonical file paths
table) are **partially superseded**. Amended form:

- Trait + DTOs: stay in `nebula-credential/src/store.rs` (not moved).
- Impls + layers + key_provider: move per the table in §2 of this ADR.
- `KeyProvider` trait move — **unchanged** from ADR-0029. ADR-0023
  supersession in location still applies to `KeyProvider` specifically.

### 6. Pending store, backup repo, and other P7 moves

ADR-0029 §2 also enumerated `pending.rs` and `backup.rs`. Those
decisions are **unaffected** by this ADR — both depend only on storage
impl concerns, not on moving the `CredentialStore` trait. They move
per the original ADR-0029 in phase P7.

## Consequences

**Positive.**

- The cargo dep-graph cycle is broken by trait/impl split, the standard
  Rust pattern for this problem (`std::io::Read` trait + many impls,
  `tokio::io::AsyncRead` trait + many impls).
- Credential's internal consumers (`resolver.rs`, `executor.rs`,
  `registry.rs`, rotation orchestration) continue to compile unchanged
  during the P6 → P8 window. Mode B pauses between phases are preserved.
- P6 scope reduces to layer-moves + KeyProvider + InMemoryStore (a
  tractable ~800-LOC PR, not a multi-thousand-LOC mega-PR).
- Consumers that only need the trait (for bounds) import one crate;
  consumers that build a composition root import two (credential +
  storage). This is the same pattern consumers already follow for
  action + credential + engine today. No new mental model.

**Negative / accepted costs.**

- ADR-0029 §Alternative A's rejection was prescient about one thing:
  "consumers import from both crates to build a layered store." That
  cost is now realized. It is, empirically, a small cost — two `use`
  lines vs one in composition-root code. Far smaller than the cost of
  resolving the cycle by other means.
- ADR-0029's main argument that "`nebula-credential` becomes a pure
  contract crate" is **softened**: credential still holds the
  `CredentialStore` trait alongside the `Credential` trait. Both are
  contracts, so "pure contract" is still accurate; the crate just has
  two contract surfaces (credential authoring + store pluggability)
  instead of one.
- The ADR series now has three documents touching credential
  architecture (0028 umbrella, 0029 main move, 0032 amendment). Fixed
  by ADR cross-refs — each cites the others.

**Neutral.**

- §12.5 primitives still live in `nebula-credential::secrets::crypto`
  (ADR-0028 invariant 1 unchanged). Storage imports via sibling dep.
- AAD bit-for-bit invariant (ADR-0028 invariant 1) is not affected —
  `EncryptionLayer` ports to storage carrying the AAD logic verbatim,
  same as the original ADR-0029 specified.
- Pending store + backup repo moves in P7 are unchanged.
- RefreshCoordinator stays in credential (ADR-0030 §3, unaffected).
- Engine + API layer ownership (ADR-0030, ADR-0031) are unaffected —
  those ADRs do not depend on where the `CredentialStore` trait lives;
  they depend on the trait existing and being callable, which it is.

## Seam / verification

- `crates/credential/Cargo.toml` — MUST NOT list `nebula-storage` in
  `[dependencies]`. CI grep gate:
  `! grep -q "^nebula-storage" crates/credential/Cargo.toml`.
- `crates/credential/src/store.rs` — retains the `CredentialStore`
  trait + DTOs. File is NOT moved in P6.
- `crates/storage/src/credential/mod.rs` — does not redefine
  `CredentialStore`; imports from `nebula_credential`.
- `cargo metadata --format-version 1` — no edge from `nebula-credential`
  to `nebula-storage`.

## Follow-ups

- Amend ADR-0028 inline (invariant 6 body) — **done in this PR**.
- Amend ADR-0029 inline (§3 re-exports section) — **done in this PR**.
- Update P6-P11 implementation plan — remove "P6.6 Update
  credential/lib.rs" surgery (no removals needed), amend P6.2 to move
  only `store_memory.rs` (not `store.rs`), amend P6.6 to skip the
  re-export block (no change needed), skip P6.10 (no internal consumer
  updates needed — all credential internal users continue to use the
  local trait). Plan amendment lands in this PR.
- P11 SDK audit task — SDK re-exports `CredentialStore` from credential
  (unchanged) and will need to add re-exports of `InMemoryStore` /
  `EncryptionLayer` / `KeyProvider` from storage for DX. Documented in
  P11.
