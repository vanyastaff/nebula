# nebula-credential
Credential storage, rotation, v2 trait-based system. Flat module structure.

## Invariants
- Credentials always encrypted at rest (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import with nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- Schemes use `fn pattern() -> AuthPattern` — not `const KIND`.

## Key Decisions
- Subfolders: `scheme/`, `credentials/`, `layer/`, `rotation/`.
- `SecretString` in nebula-core; serde via `serde_secret` / `option_serde_secret`.
- Rotation: feature-gated (`rotation`), disconnected from v2 `Credential` trait.
- `SnapshotError::SchemeMismatch.expected` is `String` (pattern has no static str).
- identity_state kinds: `"secret_token"`, `"identity_password"`.

## Key Rotation
- `EncryptedData.key_id` is `#[serde(default)]`; empty = legacy pre-rotation data.
- `EncryptionLayer::new()` uses key id `"default"`. `with_keys()` requires `current_key_id` in map (debug_assert).
- Lazy rotation on `get()`: old key_id → decrypt with old key, re-encrypt with current, `PutMode::Overwrite`.
- Legacy (`key_id == ""`): decrypt with current key + AAD fallback, then re-encrypt.

## Traps
- `into_project::<S>()` consumes snapshot — use `project::<S>()` first to verify type.
- `CredentialHandle::Clone` creates independent `ArcSwap` — share via `Arc<CredentialHandle<S>>`.
- `InMemoryStore` CAS on missing row creates instead of NotFound.
- CAS retry tests share global `AtomicU32` — race in parallel. Use `--test-threads=1`.
- `PutMode::Upsert` does not exist — use `PutMode::Overwrite` for unconditional writes.

<!-- reviewed: 2026-04-07 — Task 9: key_id field in EncryptedData, multi-key EncryptionLayer with lazy rotation. -->
