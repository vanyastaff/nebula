# nebula-credential
Universal credential management: 12 auth scheme types, open AuthScheme trait, composable storage layers, encryption key rotation, derive macros.

## Invariants
- Encrypted at rest (AES-256-GCM). `SecretString` zeroizes on drop.
- `CredentialGuard<S: Zeroize>` — Deref + Zeroize on drop + !Serialize. `new()` is `pub`; re-exported from nebula-action.
- `CredentialAccessor` trait + `ScopedCredentialAccessor` + `NoopCredentialAccessor` live here, re-exported from nebula-action.
- `CredentialAccessError` (NotFound, TypeMismatch, AccessDenied, NotConfigured). `From<_> for ActionError` maps `AccessDenied`→`SandboxViolation`, rest→`Fatal`.
- All `AuthScheme` `Debug` impls redact secrets.
- `identity_state!` calls live in scheme files, not `credentials/mod.rs`.

## Key Decisions
- Rotation: feature-gated (`rotation`), disconnected from `Credential` trait.
- `StaticProtocol` used by `#[derive(Credential)]` for unit-struct credentials. Struct-based derive deferred to v1.1.
- `design/` folder removed — specs in `docs/superpowers/specs/`.
- AAD always enforced — no legacy fallback.
- **OAuth2 AuthorizationCode typestate (issues #250+#251):** `OAuth2Config::authorization_code(redirect_uri)` takes the redirect URI as a required constructor argument and unconditionally enables PKCE S256. `AuthCodeBuilder` / `ClientCredentialsBuilder` / `DeviceCodeBuilder` are separate types — only the auth-code builder has `redirect_uri`/`pkce` methods, so it is physically impossible to build a misconfigured auth-code flow. `OAuth2Config.pkce: Option<PkceMethod>` and `OAuth2Config.redirect_uri: Option<String>` are both `Some(_)` iff `grant_type == AuthorizationCode`.
- **OAuth2Pending carries the per-flow secrets:** `pkce_verifier: Option<SecretString>`, `state: Option<String>`, `redirect_uri: Option<String>`, all `Some(_)` for AuthorizationCode. `#[serde(default)]` so records persisted before the fix still deserialize; `continue_resolve` rejects them with `InvalidInput("OAuth2 callback validation failed")`.
- **Constant-time state validation:** `continue_resolve` uses `subtle::ConstantTimeEq` on the callback `state` vs `pending.state`. Split error-message policy: `"OAuth2 state mismatch"` for the CSRF check, uniform `"OAuth2 callback validation failed"` for every other failure branch so missing-code / missing-state / missing-verifier cannot be distinguished by a probe.

## Key Rotation
- `EncryptedData.key_id` `#[serde(default)]`. `new()` registers `""` + `"default"` aliases.
- Lazy rotation on `get()`: CAS write-back, skip on VersionConflict.

## Traps
- `into_project::<S>()` consumes snapshot — use `project::<S>()` first.
- `CredentialHandle::Clone` creates independent `ArcSwap` — share via `Arc`.
- CAS retry tests race in parallel. Use `--test-threads=1`.
- `RefreshCoordinator`: Waiter path **must** pre-enable `Notified` (`pin!` + `.as_mut().enable()`) before await — else `notify_waiters()` can fire before registration. 5 s waiter timeout is intentional; store re-read is the race-recovery.

## Known Issues (deferred)
- RT-4: ScopeLayer TOCTOU on delete/put — requires trait-level conditional ops.
- RT-3: rkyv cache zeroization — not applicable yet (cache uses moka ciphertext).

<!-- reviewed: 2026-04-14 -->
