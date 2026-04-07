# nebula-credential
Credential storage, rotation, v3 universal scheme types.

## Invariants
- Encrypted at rest (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import with nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- Schemes use `fn pattern() -> AuthPattern`. 10/12 use `#[derive(AuthScheme)]`.
- `test()` returns `Result<Option<TestResult>, CredentialError>` — `Ok(None)` = not testable.

## Key Decisions
- Subfolders: `scheme/`, `credentials/`, `layer/`, `rotation/`.
- Rotation: feature-gated (`rotation`), disconnected from `Credential` trait.
- `SnapshotError::SchemeMismatch.expected` is `String`.
- Re-exports `AuthScheme` and `AuthPattern` from nebula-core.

## Key Rotation
- `EncryptedData.key_id` `#[serde(default)]`. `new()` registers `""` + `"default"` aliases.
- `with_keys()` asserts `current_key_id` in map (runtime, not debug-only).
- Lazy rotation on `get()`: CAS write-back, skip on VersionConflict.
- AAD always enforced — no fallback.

## Traps
- `into_project::<S>()` consumes snapshot — use `project::<S>()` first.
- `CredentialHandle::Clone` creates independent `ArcSwap` — share via `Arc`.
- CAS retry tests race in parallel. Use `--test-threads=1`.
- `Zeroizing<Vec<u8>>` — extract via `std::mem::take(&mut *val)`.

<!-- reviewed: 2026-04-07 — v3 review fixes: NoAuth, CAS rotation, test() Result<Option> -->
