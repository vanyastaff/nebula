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

## Traps
- `into_project::<S>()` consumes snapshot — use `project::<S>()` first to verify type.
- `CredentialHandle::Clone` creates independent `ArcSwap` — share via `Arc<CredentialHandle<S>>`.
- `InMemoryStore` CAS on missing row creates instead of NotFound.
- CAS retry tests share global `AtomicU32` — race in parallel. Use `--test-threads=1`.

<!-- reviewed: 2026-04-07 — Task 8: universal scheme types, deleted database/header_auth credentials. -->
