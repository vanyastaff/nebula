# nebula-credential
Credential storage, rotation, v2 trait-based system. Flat module structure.

## Invariants
- Credentials always encrypted at rest (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import with nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- Schemes use `fn pattern() -> AuthPattern` — not `const KIND`.
- 10 of 12 scheme types use `#[derive(AuthScheme)]`; `OAuth2Token` and `FederatedAssertion` keep manual impls (they override `expires_at()`).

## Key Decisions
- Subfolders: `scheme/`, `credentials/`, `layer/`, `rotation/`.
- `SecretString` in nebula-core; serde via `serde_secret` / `option_serde_secret`.
- Rotation: feature-gated (`rotation`), disconnected from v2 `Credential` trait.
- `SnapshotError::SchemeMismatch.expected` is `String` (pattern has no static str).
- identity_state kinds: `"secret_token"`, `"identity_password"`.

## Key Rotation
- `EncryptedData.key_id` `#[serde(default)]`; empty = unreadable (hard error, no fallback).
- `EncryptionLayer::new()` → key id `"default"`. `with_keys()` requires `current_key_id` in map.
- Lazy rotation on `get()`: old key_id → decrypt + AAD + re-encrypt with current (`PutMode::Overwrite`).
- AAD always enforced — no-AAD data rejected with `StoreError::Backend`.

## Traps
- `into_project::<S>()` consumes snapshot — use `project::<S>()` first to verify type.
- `CredentialHandle::Clone` creates independent `ArcSwap` — share via `Arc<CredentialHandle<S>>`.
- `InMemoryStore` CAS on missing row creates instead of NotFound.
- CAS retry tests share global `AtomicU32` — race in parallel. Use `--test-threads=1`.
- `PutMode::Upsert` does not exist — use `PutMode::Overwrite`.
- `Zeroizing<Vec<u8>>` has no `into_inner()` — extract via `std::mem::take(&mut *val)`.

<!-- reviewed: 2026-04-07 — Task 12-13: AuthScheme derive macro + scheme migration. -->
