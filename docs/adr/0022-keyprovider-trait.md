---
id: 0022
title: keyprovider-trait
status: proposed
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [credential, security, encryption, key-custody, composition-root, canon-12.5]
related:
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/STYLE.md#6-secret-handling
  - docs/adr/0020-library-first-gtm.md
  - docs/adr/0021-crate-publication-policy.md
  - docs/audit/2026-04-19-codebase-quality-audit.md
  - crates/credential/src/layer/encryption.rs
  - crates/credential/src/crypto.rs
linear: []
---

# 0022. `KeyProvider` trait between `EncryptionLayer` and key material source

## Context

[`crates/credential/src/layer/encryption.rs:89`](../../crates/credential/src/layer/encryption.rs:89)
constructs `EncryptionLayer` by taking `Arc<EncryptionKey>` directly. There is
no seam between the composition root and the key material: every caller must
materialise an `EncryptionKey` in process memory and hand it to the layer.
`with_keys` at `encryption.rs:108` extends this to multi-key rotation, but
the shape is the same — raw `Arc<EncryptionKey>` values in, no abstraction
over the source.

The 2026-04-19 codebase quality audit's `security-lead` section
([`docs/audit/2026-04-19-codebase-quality-audit.md §Findings — security-lead`](../audit/2026-04-19-codebase-quality-audit.md))
named this as the load-bearing gate before any `apps/server` composition
work:

> `crates/credential/src/layer/encryption.rs:62` accepts `Arc<EncryptionKey>`
> directly — there is no `KeyProvider` seam. Any composition path (library
> embedder, future `apps/server`, tests) must load the key into process
> memory with unknown provenance. Once `apps/server` ships with env-only
> key loading, that becomes de facto API forever (operators write systemd
> units, configs, runbooks). **Must land before any composition PR**.

[ADR-0020](./0020-library-first-gtm.md) §3 pre-condition 1 makes that
finding normative: no `apps/server` PR merges until this seam exists.
Without it, the first server PR would load `EncryptionKey` from an env
var inline, operators would build systemd units and runbooks around that
exact shape, and file / KMS / Vault / cloud-secret-manager integrations
would become deprecation pain instead of additive implementations.

[ADR-0021](./0021-crate-publication-policy.md) §3 names `nebula-credential`
as one of the initial published crates, citing "the `KeyProvider` seam
from the 2026-04-19 audit's `security-lead` section is an intentional
public contract." This ADR is the contract.

Three existing constraints shape the seam:

1. **[`docs/STYLE.md §6 — Secret handling`](../STYLE.md#6-secret-handling)**
   mandates `Zeroize` / `ZeroizeOnDrop` on secret material, redacted
   `Debug` / `Display`, `Zeroizing<Vec<u8>>` on intermediate plaintext,
   and "no secret in error strings." The trait must not relax any of
   these.
2. **AAD binding at [`encryption.rs:146-211`](../../crates/credential/src/layer/encryption.rs:146)
   is correct today** (credential ID is AAD; record-swapping is rejected;
   AAD-less records are rejected). The trait must not touch this path.
3. **Canon [§12.5 — Secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth)**
   invariants — authenticated encryption at rest, no bypass for debugging,
   zeroize everywhere — are load-bearing. The seam must preserve them.

## Decision

### 1. A `KeyProvider` trait sits between `EncryptionLayer` and key material

```rust
pub trait KeyProvider: Send + Sync + 'static {
    /// Return the current encryption key — the key `EncryptionLayer` will
    /// use for new writes and for decrypting records tagged with the
    /// current `version()`.
    ///
    /// The returned `Arc<EncryptionKey>` is a stable handle; callers may
    /// hold it for the duration of a single operation without copying the
    /// secret bytes.
    fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError>;

    /// A stable identifier for the current key, used as
    /// [`EncryptedData::key_id`](crate::crypto::EncryptedData) in new
    /// envelopes and for rotation correlation across providers and logs.
    /// Must be non-empty; the encryption layer's envelope writer rejects
    /// empty `key_id`s.
    fn version(&self) -> &str;
}
```

Required properties:

- **`Send + Sync + 'static`** so `Arc<dyn KeyProvider>` threads through
  the composition root without lifetime gymnastics.
- **Errors are typed** via `ProviderError` (see §3). No `String` /
  `anyhow` in the trait signature; `nebula-credential` is a library
  crate and follows `docs/STYLE.md §4 — Error taxonomy`.
- **No key material in `Debug` / `Display`** on either the trait, its
  error type, or any in-tree impl. `EncryptionKey` is already
  `ZeroizeOnDrop` and owns its `Debug`; providers that wrap it must
  redact their own fields.

### 2. `EncryptionLayer` takes `Arc<dyn KeyProvider>`, not `Arc<EncryptionKey>`

`new` and `with_keys` at [`encryption.rs:89,108`](../../crates/credential/src/layer/encryption.rs:89)
are replaced by:

```rust
impl<S> EncryptionLayer<S> {
    pub fn new(inner: S, key_provider: Arc<dyn KeyProvider>) -> Self;

    pub fn with_legacy_keys(
        inner: S,
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self;
}
```

The layer queries the provider on every read and write (`current_key()`
+ `version()`); the lazy-rotation path at
[`encryption.rs:232-254`](../../crates/credential/src/layer/encryption.rs:232)
consults the provider when deciding whether to re-encrypt. When the
provider starts returning a new key (operator rotated in-place, or a
future KMS impl refreshed its cache), the next read picks that up
without restart.

`legacy_keys` is decrypt-only: records whose envelope `key_id` matches
an entry are decrypted with that entry, then re-encrypted with the
current key (lazy rotation — canon §12.5 invariant unchanged).
Operators holding rotation history — e.g. the `""` → `"default"`
migration documented at [`encryption.rs:66-88`](../../crates/credential/src/layer/encryption.rs:66) —
register it via `with_legacy_keys` instead of `with_keys`.

### 3. Three in-tree implementations

#### `EnvKeyProvider` — canonical local / single-tenant default

Reads a 32-byte AES-256 key from `NEBULA_CRED_MASTER_KEY` (base64-encoded).
Fail-closed validation mirrors
[`JwtSecret::new`](../../crates/api/src/config.rs:55) exactly:

- Missing env var → `ProviderError::NotConfigured { name: "NEBULA_CRED_MASTER_KEY" }`.
- Dev placeholder literal → `ProviderError::DevPlaceholder`.
- Short or wrong-length decode → `ProviderError::KeyMaterialRejected { reason }`.
- Base64 decode failure → `ProviderError::Decode { .. }`.

Intermediate plaintext (the base64 string, the decoded bytes) is held in
`Zeroizing<T>` so scope exit scrubs it per `STYLE.md §6`. The
`EncryptionKey` newtype handles its own `ZeroizeOnDrop` once constructed.

`version()` returns `"env:<fingerprint>"` where `<fingerprint>` is the
first 8 bytes of SHA-256 over the key material, hex-encoded
(16 chars). **Rotating the env var to different bytes automatically
changes the fingerprint**, so the envelope `key_id` differs from the
pre-rotation state — existing records flow through the `with_legacy_keys`
path instead of being treated as "current" and silently mis-decrypting
under the new key. This is the rotation-safety invariant captured on
the trait `version()` docstring: any provider that hand-manages the
version string must preserve it.

#### `FileKeyProvider` — self-hosted / mounted-secret default

Reads 32 raw key bytes from a path. Refuses world-readable files on Unix
(`mode & 0o004 != 0` → `ProviderError::InsecurePermissions { path }`);
the check is skipped under `#[cfg(not(unix))]` because POSIX mode bits
do not apply to Windows.

Filesystem failures (missing file, permission denied, …) surface as
`ProviderError::FileIo { path, source }` so the offending path is
available for diagnostics without correlating against log lines —
`std::fs` errors do not usually include the path themselves. Non-file
backing sources (future providers) use `ProviderError::Io { source }`.

Useful for Kubernetes secrets mounted into the container filesystem
(`/run/secrets/`), systemd credential files (`$CREDENTIALS_DIRECTORY/`),
and operators who want the key on-disk with a discrete rotation story
rather than in the process environment.

`version()` returns `"file:<filename>:<fingerprint>"` — filename keeps
logs readable; fingerprint (same SHA-256 prefix scheme as
`EnvKeyProvider`) makes **in-place rewrites observable**. Kubernetes
secret rewrites and systemd credential refreshes keep the path stable
but change the bytes; the fingerprint segment changes accordingly and
pre-rotation records flow through `with_legacy_keys`.

#### `StaticKeyProvider` — test-only, gated behind `test-util`

`#[cfg(any(test, feature = "test-util"))]`. Wraps an in-memory
`Arc<EncryptionKey>` and a caller-supplied version string. The feature
is not a `default` feature; production release builds never see it.
Every non-test call site in the workspace — production, examples,
`apps/cli` — uses `EnvKeyProvider` or `FileKeyProvider`.

### 4. Security-lead audit verdict

This ADR discharges the finding recorded in
[`docs/audit/2026-04-19-codebase-quality-audit.md §Findings — security-lead`](../audit/2026-04-19-codebase-quality-audit.md).
ADR-0020 §3 pre-condition 1 is satisfied once this ADR + its impl PR
merge; the `apps/server` follow-up ADRs unblock accordingly. Follow-up
KMS / Vault / cloud-secret-manager impls (see §Alternatives considered)
are additive: a new type implements `KeyProvider` and plugs into any
existing `EncryptionLayer::new` call site.

### 5. SemVer commitment (ADR-0021 alignment)

`nebula-credential` is in the initial published set per
[ADR-0021 §3](./0021-crate-publication-policy.md). From the day this
ADR's implementation lands, `KeyProvider`, `ProviderError`, and the
three in-tree impls are part of the crate's public SemVer surface:

- Breaking the trait signature requires a superseding ADR.
- `ProviderError` carries `#[non_exhaustive]` so new variants (e.g. KMS
  transport errors) are additive.
- `EnvKeyProvider::ENV_VAR`, `MIN_BYTES`, and `DEV_PLACEHOLDER` are
  public associated consts — changing them is a breaking change for
  operators' runbooks, and is treated as such.
- The SHA-256-prefix `version()` scheme used by `EnvKeyProvider` and
  `FileKeyProvider` is **observable SemVer surface**: the
  `"env:<fp>"` / `"file:<name>:<fp>"` shape appears in stored envelope
  `key_id`s and in operator logs. Changing the hash length or prefix
  layout is a breaking change — operators register legacy keys by the
  string they previously saw in logs. Future providers are free to
  pick a different scheme; changing either of these two is not.

### 6. Rotation procedure

When an operator rotates a key handled by `EnvKeyProvider` or
`FileKeyProvider`, the fingerprint-backed `version()` turns the rotation
into a loud failure (decryption error naming the missing key id) rather
than a silent mis-decrypt. The happy-path sequence is:

1. **Before retiring the old key**, record its `version()` — typically
   emitted on startup or visible in metrics. The fingerprint suffix
   (16 hex chars after the `env:` / `file:<name>:` prefix) is the
   stable identifier for the rotation.
2. **Swap the key** — rewrite `NEBULA_CRED_MASTER_KEY` or the
   file. Do not restart yet.
3. **Register the old key as legacy** at restart so pre-rotation
   records remain readable:

   ```rust
   let old_key = Arc::new(EncryptionKey::from_bytes(old_bytes));
   let provider = Arc::new(EnvKeyProvider::from_env()?);
   let layer = EncryptionLayer::with_legacy_keys(
       store,
       provider,
       vec![(old_version_string, old_key)], // e.g. "env:a3f2c891b4e5d267"
   );
   ```

4. Lazy rotation re-encrypts records on their next read (per the
   existing module semantics at
   [`encryption.rs:232-254`](../../crates/credential/src/layer/encryption.rs:232)).
   Once records converge — observable via future rotation-completion
   telemetry — the legacy entry can be dropped on the next restart.

Operators who **forget** step 3 get `StoreError::Backend("encryption
key '<old-version>' not found")` on the next credential read instead of
corrupted plaintext. That loud-failure mode is the entire point of the
fingerprint scheme.

## Consequences

**Positive**

- The load-bearing `apps/server` pre-condition from ADR-0020 §3 is
  satisfied without freezing env-only as the de-facto API. When
  `apps/server` lands, it consumes `Arc<dyn KeyProvider>` from its
  composition root; the operator chooses `Env` / `File` / (future KMS)
  at wiring time, not at library-compile time.
- The fail-closed shape of `EnvKeyProvider` matches `JwtSecret` exactly,
  so a single mental model covers both pre-conditions named in
  ADR-0020 §3 (auth secret and encryption key).
- KMS / Vault / cloud-secret-manager backends become trivially additive
  — each is a new type implementing `KeyProvider`. The audit's "Open
  ADRs needed" line for KMS-adjacent seams is unblocked without
  committing to any specific cloud-secret surface in this PR.
- `with_legacy_keys` preserves the `""` → `"default"` migration path
  documented at [`encryption.rs:66-88`](../../crates/credential/src/layer/encryption.rs:66)
  without change to the operator-facing envelope semantics.

**Negative / accepted costs**

- `EncryptionLayer::new`'s signature is a breaking change for any
  out-of-tree caller. At the time this ADR lands, the workspace has
  zero production call sites — the only callers inside the crate are
  the `#[cfg(test)]` module at
  [`encryption.rs:257+`](../../crates/credential/src/layer/encryption.rs:257),
  which move to `StaticKeyProvider` in the same PR. External consumers
  are none per ADR-0021 §Context; the cost is absorbed before
  `nebula-credential` has an external audience.
- Provider construction is synchronous (§Alternatives considered C).
  KMS impls that require network calls must cache the key eagerly at
  construction (the universal pattern — doing a network round-trip per
  credential read would dominate the encryption hot path) and refresh
  on a background task or on an explicit `refresh()` hook added later
  via a superseding ADR if the need arises. This is not a regression
  against the current `Arc<EncryptionKey>` shape, which is equally
  eager.
- Two constructors (`new`, `with_legacy_keys`) instead of one. The
  surface is still narrow; `with_keys`'s old three-argument signature
  had the same count and was already documented.

**Neutral**

- The AAD binding path is untouched. Lazy rotation still re-encrypts
  old-key records with the current key on read, via the same
  `decrypt_possibly_rotating` code at
  [`encryption.rs:232-254`](../../crates/credential/src/layer/encryption.rs:232).
- Canon §12.5 — authenticated encryption at rest, `Zeroize` /
  `ZeroizeOnDrop`, redacted `Debug` — is preserved. §6 of STYLE.md
  applies unchanged to provider impls (redacted `Debug`, typed errors,
  `Zeroizing` intermediate buffers).

## Alternatives considered

### A. KMS / Vault / cloud-secret-manager impl in the same PR

Rejected. Each cloud secret manager is its own surface (IAM role model,
transit-encryption endpoints, audit shape, cache behaviour, failure
taxonomy). Committing to one inside this ADR would balloon the PR,
force premature opinions on at least one cloud we may not actually
need, and obscure the trait's simplicity. Ship the trait; each backend
earns its own follow-up ADR (`KmsKeyProvider`, `VaultKeyProvider`, etc).

### B. Provider owns the full key map (current + legacy)

Considered: `KeyProvider::keys()` returns a `HashMap<String, Arc<EncryptionKey>>`
for both current and historical decryption keys. Rejected. The provider
is the source of the **current** key — a KMS-backed provider cannot
reasonably enumerate "every key that ever wrote a record in this
deployment." Legacy keys are a deployment-scoped decrypt capability,
not a provider concern; keeping them on the layer (`with_legacy_keys`)
matches the existing rotation model and keeps each KMS impl's surface
minimal.

### C. Async `current_key()` via `#[dynosaur::dynosaur(DynKeyProvider)]`

Considered: make `current_key()` `async`, use the `dynosaur` pattern
from [ADR-0014](./0014-dynosaur-macro.md) for `dyn` compatibility.
Rejected for this ADR. Every in-tree impl is synchronous — `Env` /
`File` read the key at construction and return `Arc::clone`; `Static`
is pure memory. Future KMS impls universally cache the current key in
memory (a network round-trip per credential read would dominate the
encryption hot path), so their `current_key()` is also effectively
`Arc::clone`. A synchronous trait:

- Is naturally `dyn`-compatible — `Arc<dyn KeyProvider>` without
  `dynosaur` on the call path, which keeps this ADR's surface minimal
  and does not introduce a workspace dependency on `dynosaur` out of
  the sequenced migration described in the audit's P2 #18.
- Matches the shape of the existing `CredentialStore` trait at
  [`store.rs:88-141`](../../crates/credential/src/layer/../store.rs:88)
  for read-paths the engine takes on the hot loop.

If a future ADR introduces a provider that genuinely cannot cache
(e.g., short-lived KMS data keys with derivation-per-call semantics),
that ADR can supersede §1's signature. Until then, synchronous is the
honest shape.

### D. Keep `Arc<EncryptionKey>` at the constructor; add a separate
`KeyProviderExt` trait for "advanced" composition

Rejected. Two parallel public surfaces (raw key + provider trait) is
the worst outcome: operators would pick either by accident, and the
whole point of the seam — preventing env-only from becoming the de-facto
API — is lost. The trait replaces the raw-key constructor.

### E. No ADR; ship the trait as an implementation detail

Rejected. `nebula-credential` is a published crate per ADR-0021 §3,
and ADR-0020 §3 names this trait as a pre-condition to `apps/server`.
Both contexts make this an explicit canon-level decision, not an
implementation detail.

## Follow-ups

- **`apps/server` composition PR** unblocks on this ADR + its
  implementation. ADR-0020 §3 pre-condition 1 is satisfied once
  `EncryptionLayer::new` takes `Arc<dyn KeyProvider>`.
- **`KmsKeyProvider` ADR** — earliest cloud-secret-manager backend.
  Expected to propose AWS KMS or GCP KMS with a narrow surface
  (data-key envelope encryption, IAM-role-bound, single-region). Out
  of scope for this ADR; will cite this one.
- **`VaultKeyProvider` ADR** — HashiCorp Vault Transit secrets engine
  backend for self-hosted deployments that already operate Vault. Out
  of scope for this ADR.
- **Key-rotation runbook** under `docs/` explaining the
  `with_legacy_keys` / lazy-rotation flow operators follow when
  turning over `NEBULA_CRED_MASTER_KEY`. Documentation-only follow-up.

## Seam / verification

- **Trait site.** [`crates/credential/src/layer/key_provider.rs`](../../crates/credential/src/layer/key_provider.rs)
  declares the `KeyProvider` trait, `ProviderError` enum, and the three
  in-tree impls. Redacted `Debug` is written by hand on every provider
  that holds key material (STYLE.md §6.2 rule 3).
- **Constructor site.** [`crates/credential/src/layer/encryption.rs`](../../crates/credential/src/layer/encryption.rs)
  `new` / `with_legacy_keys` take `Arc<dyn KeyProvider>`; the
  `current_key_id` is `self.key_provider.version()` on every
  envelope read and write. AAD, lazy rotation, and CAS-on-rotate
  (GitHub issue #282) behaviour is unchanged.
- **Unit tests.**
  - Trait-level test (`key_provider.rs` tests module): a counting
    provider verifies `current_key()` is called on `put` and `get`,
    i.e. rotation triggers a re-fetch rather than a cached-at-
    construction snapshot.
  - `EnvKeyProvider`: dev-placeholder / short / wrong-length / decode
    failures all return the correct typed `ProviderError` variant
    (in-crate, via `from_base64`). `from_env` missing-var coverage
    lives in a separate integration test binary because env-var
    mutation requires `unsafe` which the crate forbids. Rotation-
    safety is pinned by two tests: `version_changes_with_key` (two
    different keys → different versions) and
    `version_stable_for_same_key` (repeat construction → same version).
    Mirrors [`config.rs` `jwt_secret_*` tests](../../crates/api/src/config.rs:407).
  - `FileKeyProvider`: `#[cfg(unix)]` test refuses a `0o644` key file;
    valid 32-byte file decodes to an `EncryptionKey` with a
    fingerprint-tailed `version()`; missing-file errors surface as
    `ProviderError::FileIo { path, .. }` with the path preserved;
    `version_changes_with_content` rewrites the same path and asserts
    the version changes, mirroring the Kubernetes / systemd in-place
    rewrite flow.
  - `StaticKeyProvider`: round-trip + version preservation.
- **Integration.** The existing `EncryptionLayer` test coverage at
  [`encryption.rs:257+`](../../crates/credential/src/layer/encryption.rs:257)
  — round-trip, AAD enforcement, multi-key lazy rotation, CAS-on-
  rotate (#282), and the `""` → `"default"` legacy-alias refusal
  (#281) — is ported to `StaticKeyProvider` in the same PR and
  preserves every invariant. `StoreError::Backend` wrapping of
  `ProviderError` is exercised by the counting-provider failure-
  injection test.

Related ADRs:

- [ADR-0020](./0020-library-first-gtm.md) — pre-condition 1 is this ADR.
- [ADR-0021](./0021-crate-publication-policy.md) — names
  `nebula-credential` and the `KeyProvider` seam as an intentional
  public contract.
- [ADR-0014](./0014-dynosaur-macro.md) — cited in Alternative C for
  why sync is honest for this surface.
