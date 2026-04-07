# nebula-credential
Credential storage, rotation, v2 trait-based system. Flat module structure.

## Invariants
- Credentials always encrypted at rest (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import with nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- Error hierarchy: `CredentialError` + `StoreError` + `SnapshotError`. No `StorageError`/`ManagerError`.
- `CredentialSnapshot` carries `Box<dyn Any + Send + Sync>`. Fields private — use `project::<S>()`/`into_project::<S>()`.

## Key Decisions
- Subfolders only: `scheme/`, `credentials/`, `layer/`, `rotation/`. No `utils/` or `core/`.
- `SecretString` lives in nebula-core; re-exported here. Serde: `nebula_core::serde_secret` / `nebula_core::option_serde_secret`.
- Schemes use `fn pattern() -> AuthPattern` — not `const KIND`.
- Rotation module: feature-gated (`rotation`), disconnected from v2 `Credential` trait.
- `OAuth2State.auth_style` preserved from initial exchange for correct refresh.

## Traps
- `CredentialSnapshot::into_project::<S>()` consumes self — type mismatch loses it. Use `project::<S>()` (borrow) first.
- `CredentialHandle::Clone` creates independent `ArcSwap` — clones don't see rotation updates. Share via `Arc<CredentialHandle<S>>`.
- `verify_owner` fail-closed: credentials without `owner_id` in metadata are admin-only.
- `InMemoryStore` CAS on missing row creates instead of NotFound (B10, open).
- `bearer.rs` still uses `const KIND` — fails to compile until Task 7 wires `scheme/mod.rs`.
- `option_serde_secret` module exists in nebula-core (credential-v3 branch) for `Option<SecretString>` fields.

<!-- reviewed: 2026-04-07 — Task 3-5: 10 new scheme files + certificate.rs (CertificateAuth→Certificate, new fields) + oauth2.rs → fn pattern(). -->
