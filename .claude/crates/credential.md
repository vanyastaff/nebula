# nebula-credential
Universal credential management: 12 auth scheme types, open AuthScheme trait, composable storage layers, encryption key rotation, derive macros.

## Invariants
- Encrypted at rest (AES-256-GCM). `SecretString` zeroizes on drop.
- `CredentialGuard<S: Zeroize>` — Deref + Zeroize on drop + !Serialize. `new()` is `pub` — action crate constructs guards in context methods. Re-exported from nebula-action for backward compat.
- `CredentialAccessor` trait, `ScopedCredentialAccessor`, `NoopCredentialAccessor`, `default_credential_accessor()` live here. Re-exported from nebula-action for backward compat.
- `CredentialAccessError` (NotFound, TypeMismatch, AccessDenied, NotConfigured) — error type for accessor methods. `From<CredentialAccessError> for ActionError` in nebula-action maps `AccessDenied` to `SandboxViolation`, others to `Fatal`.
- No direct import with nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- `identity_state!` calls live in scheme files, not `credentials/mod.rs`.

## Key Decisions
- Rotation: feature-gated (`rotation`), disconnected from `Credential` trait.
- `StaticProtocol` used by `#[derive(Credential)]` for unit-struct credentials. Struct-based derive deferred to v1.1.
- `design/` folder removed — specs in `docs/superpowers/specs/`.
- AAD always enforced — no legacy fallback.

## Key Rotation
- `EncryptedData.key_id` `#[serde(default)]`. `new()` registers `""` + `"default"` aliases.
- Lazy rotation on `get()`: CAS write-back, skip on VersionConflict.

## Traps
- `into_project::<S>()` consumes snapshot — use `project::<S>()` first.
- `CredentialHandle::Clone` creates independent `ArcSwap` — share via `Arc`.
- CAS retry tests race in parallel. Use `--test-threads=1`.

## Known Issues (deferred)
- RT-4: ScopeLayer TOCTOU on delete/put — requires trait-level conditional ops.
- RT-3: rkyv cache zeroization — not applicable yet (cache uses moka ciphertext).

<!-- reviewed: 2026-04-08 -->
