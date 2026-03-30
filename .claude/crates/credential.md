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
- `RefreshCoordinator`: winner refreshes, waiters block on `Notify`. `Winner(Arc<Notify>)` + `scopeguard` ensures waiters are woken on any exit (panic, timeout, error). `complete()` removes the in-flight entry. Circuit breaker: 5 failures in 5 min opens circuit, skips refresh and serves stale. Waiter timeout: 60s max wait on `Notify`. Framework timeout: 30s hard limit on `C::refresh()` calls.
- `CredentialResolver::resolve_with_refresh()` uses `REFRESH_POLICY.early_refresh` (default 5 min) to refresh **before** expiry, not after.
- `CredentialContext` carries optional `callback_url`, `app_url`, `session_id` (private, with builder + accessors) for interactive OAuth2/SAML flows.
- `SecretString` serializes as `"[REDACTED]"` — tests must construct raw JSON for round-trip.
- `PendingToken`: 32-byte CSPRNG, URL-safe base64 no-pad, `Display` redacts. `generate()` is `pub(crate)` — used by `PendingStateStore` impls.
- `PendingStateStore` trait: 4-dimensional token binding (credential_kind, owner_id, session_id, token_id). Separate from `CredentialStore` — different lifecycle (minutes vs years), TTL-enforced, single-use consume.

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- Storage providers feature-gated: `storage-local`, `-aws`, `-postgres`, `-vault`, `-k8s`.

## Relations
- Depends on: nebula-core, nebula-eventbus. Peer: nebula-resource.

- `CredentialError` v2 variants: `InvalidInput`, `RefreshFailed` (with `RefreshErrorKind` + `RetryAdvice`), `RevokeFailed`, `CompositionNotAvailable`, `CompositionFailed`, `SchemeMismatch`. Supporting enums: `RefreshErrorKind`, `RetryAdvice`, `ResolutionStage` — all `#[non_exhaustive]`.

- v2 built-in credentials have enriched `parameters()` with descriptions, placeholders, and defaults from v1 protocol logic. `DatabaseCredential`: port defaults to 5432, `ssl_mode` defaults to "prefer" and is parsed into `SslMode` enum. `ApiKeyCredential`: optional `server` URL field. `SslMode` re-exported from crate root.
- Scheme coercion impls in `scheme/coercion.rs`: `From<OAuth2Token> for BearerToken` (infallible), `TryFrom<ApiKeyAuth> for BearerToken` (Authorization header only, case-insensitive), `TryFrom<SamlAuth> for BearerToken` (requires assertion_b64). Errors use `CredentialError::SchemeMismatch`.
- `executor.rs`: framework-level `execute_resolve`/`execute_continue` free functions. 30s hard timeout via `tokio::time::timeout`. Handles `PendingState` lifecycle (put/consume via `PendingStateStore`). Returns `ResolveResponse<S>` (Complete/Pending/Retry). `ExecutorError` wraps timeout, credential, and store errors.

<!-- reviewed: 2026-03-30 — scheme coercion module added -->
<!-- reviewed: 2026-03-30 — 13 AuthScheme types complete: added HeaderAuth, HmacSecret, SshAuth, CertificateAuth, AwsAuth, LdapAuth, SamlAuth, KerberosAuth -->
<!-- updated: 2026-03-25 — polish v2 module names, rename types -->
<!-- reviewed: 2026-03-30 — absorbed auth RFCs into plans/, auth crate deleted -->
