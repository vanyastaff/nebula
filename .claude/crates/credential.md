# nebula-credential
Credential storage, manager, rotation, protocols. v2 rewrite in progress alongside v1.

## Invariants
- Credentials **always encrypted at rest** (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import between nebula-credential and nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.

## Key Decisions
- `CredentialProvider` = DI for actions; never inject `CredentialManager` directly.
- v2 coexists with v1. RPITIT, no `#[async_trait]`. `CredentialStateV2` keeps V2 suffix (v1 conflict).
- `CredentialStore`/`CredentialRegistry` renamed from V2 suffixed names. Files: `credential_trait.rs`, `credential_handle.rs`, `credential_registry.rs`, `credential_store.rs`.
- `CredentialHandle` uses `ArcSwap<S>` — `snapshot()` returns `Arc<S>`, `replace()` (pub(crate)) enables hot-swap by `RefreshCoordinator`. Clone creates independent `ArcSwap` with same underlying `Arc`.
- `EncryptionLayer<S>` serializes `EncryptedData` as JSON bytes in `data` field.
- `RefreshCoordinator`: winner refreshes, waiters block on `Notify`. `Winner(Arc<Notify>)` + `scopeguard` ensures waiters are woken on any exit (panic, timeout, error). `complete()` removes the in-flight entry.
- `CredentialResolver::resolve_with_refresh()` uses `REFRESH_POLICY.early_refresh` (default 5 min) to refresh **before** expiry, not after.
- `CredentialContext` carries optional `callback_url`, `app_url`, `session_id` (private, with builder + accessors) for interactive OAuth2/SAML flows.
- `SecretString` serializes as `"[REDACTED]"` — tests must construct raw JSON for round-trip.

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- Storage providers feature-gated: `storage-local`, `-aws`, `-postgres`, `-vault`, `-k8s`.

## Relations
- Depends on: nebula-core, nebula-eventbus. Peer: nebula-resource.

- `CredentialError` v2 variants: `InvalidInput`, `RefreshFailed` (with `RefreshErrorKind` + `RetryAdvice`), `RevokeFailed`, `CompositionNotAvailable`, `CompositionFailed`, `SchemeMismatch`. Supporting enums: `RefreshErrorKind`, `RetryAdvice`, `ResolutionStage` — all `#[non_exhaustive]`.

<!-- reviewed: 2026-03-30 — AuthScheme extended with KIND, expires_at, serde bounds; all 5 scheme impls updated -->
<!-- updated: 2026-03-25 — polish v2 module names, rename types -->
<!-- reviewed: 2026-03-30 — absorbed auth RFCs into plans/, auth crate deleted -->
