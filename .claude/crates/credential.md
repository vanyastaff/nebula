# nebula-credential
Credential storage, rotation, v2 trait-based system. v1 modules fully deleted.

## Invariants
- Credentials **always encrypted at rest** (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import between nebula-credential and nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.

## Key Decisions
- v1 modules deleted: `traits/`, `protocols/`, `providers/`, `manager/`, `any.rs` (v1 blanket), `core/state.rs`, `core/filter.rs`, `core/status.rs`, `core/reference.rs`, `core/result.rs`. ~19K LOC removed.
- `CredentialStateV2` renamed to `CredentialState` (no v1 conflict anymore).
- `CredentialState` trait: `Serialize + DeserializeOwned + Send + Sync + 'static` with `KIND` and `VERSION` consts. Replaces v1 `CredentialState` (which was `Serialize + Deserialize + Send + Sync + Clone`).
- `OAuth2Config`, `GrantType`, `AuthStyle` moved from deleted `protocols/oauth2/config.rs` to `credentials/oauth2_config.rs`.
- `TestableCredential` and `RotatableCredential` traits moved from deleted `traits/` into `rotation/validation.rs` (they're rotation-specific).
- `AnyCredential` trait rewritten: blanket impl on v2 `Credential` (was on v1 `CredentialType`).
- `CredentialSnapshot` kept in `core/snapshot.rs` — used by `nebula-action` for passing credential data to actions.
- `#[derive(Credential)]` macro generates v2 `Credential` impl from `#[credential(key, name, scheme, protocol)]` attrs. Delegates to `StaticProtocol`. State = Scheme identity path, NoPendingState.
- `CredentialHandle` uses `ArcSwap<S>` — `snapshot()` returns `Arc<S>`, `replace()` (pub(crate)) enables hot-swap by `RefreshCoordinator`.
- `EncryptionLayer<S>` uses credential ID as AAD in AES-256-GCM. Decrypt falls back to no-AAD for backward compat.
- `AuditLayer<S>` logs credential access metadata via pluggable `AuditSink` trait. Never sees plaintext.
- `CacheLayer<S>` wraps any `CredentialStore` with moka LRU+TTL cache. Caches ciphertext.
- `ScopeLayer<S>` outermost layer — multi-tenant isolation via `ScopeResolver` trait.
- `RefreshCoordinator`: winner refreshes, waiters block on `Notify`. Circuit breaker: 5 failures in 5 min.
- `CredentialResolver::resolve_with_refresh()` refreshes before expiry using `REFRESH_POLICY.early_refresh`.
- `SecretString` serializes as `"[REDACTED]"` — tests must construct raw JSON for round-trip.
- `PendingStateStore` trait: 4-dimensional token binding (credential_kind, owner_id, session_id, token_id).

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- Storage providers feature-gated: `storage-local`, `-aws`, `-postgres`, `-vault`, `-k8s`.
- v1 prelude deleted — crates using `nebula_credential::prelude::*` must switch to explicit v2 imports.
- `#[derive(Credential)]` requires `identity_state!` invocation for the scheme type separately — macro doesn't generate it.

## Relations
- Depends on: nebula-core, nebula-eventbus. Peer: nebula-resource.
- `core/` module: `CredentialContext`, `CredentialDescription`, `CredentialError`, `CredentialMetadata`, `CredentialSnapshot`, `CredentialId`.
- v2 built-in credentials: `ApiKeyCredential`, `BasicAuthCredential`, `DatabaseCredential`, `HeaderAuthCredential`, `OAuth2Credential`.
- Rotation module: policy, transaction, blue-green, validation (with `TestableCredential`/`RotatableCredential`), retry, backup, events, metrics.

- `CredentialKey` newtype: thin `&'static str` wrapper for credential type identifiers. Non-breaking addition — `Credential::KEY` still uses `&'static str`, `CredentialKey` available for gradual adoption. `credential_key!` macro for compile-time construction.
- `StaticProtocol` trait: `parameters() + build(values) → Scheme` pattern for non-interactive credentials. `#[derive(Credential)]` will consume this to auto-generate `resolve()`.

- `utils::serde_base64` module: shared base64 serde helpers for `Vec<u8>` fields. Used by all store backends (local, postgres, aws, k8s).
- `credential_store::test_helpers::make_credential()`: shared `#[cfg(test)]` helper for constructing `StoredCredential` in tests.
- `OAuth2Pending.grant_type` is `GrantType` enum (was stringly-typed `String`).
- `oauth2_flow` shares a single `reqwest::Client` via `OnceLock` across all token exchange functions.
- `EncryptionLayer::put()` saves plaintext data before encrypting to avoid redundant decryption on return.
- `RefreshCoordinator` uses named `FailureRecord` struct instead of `(u32, Instant)` tuple.

<!-- updated: 2026-03-30 — polished v2 code from review findings -->
