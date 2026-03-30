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
- `AuditLayer<S>` logs credential access metadata (id, operation, result, timestamp) via pluggable `AuditSink` trait. Never sees plaintext — sits between `ScopeLayer` and `EncryptionLayer`. Uses `"*"` as credential_id for list ops. Maps `AlreadyExists`/`VersionConflict` to `AuditResult::Conflict`.
- `CacheLayer<S>` wraps any `CredentialStore` with moka LRU+TTL cache. Caches **ciphertext** (sits below `EncryptionLayer`). Invalidates on put/delete. `list()` passes through uncached. `exists()` checks cache first.
- `ScopeLayer<S>` outermost layer — multi-tenant isolation via `ScopeResolver` trait. Checks `metadata["owner_id"]` on get/delete, injects on put. `None` owner = admin bypass. Mismatch returns `NotFound` (no existence leak). `list()`/`exists()` pass through (backend filtering needed for full isolation).
- `RefreshCoordinator`: winner refreshes, waiters block on `Notify`. `Winner(Arc<Notify>)` + `scopeguard` ensures waiters are woken on any exit (panic, timeout, error). `complete()` removes the in-flight entry. Circuit breaker: 5 failures in 5 min opens circuit, skips refresh and serves stale. Waiter timeout: 60s max wait on `Notify`. Framework timeout: 30s hard limit on `C::refresh()` calls.
- `CredentialResolver::resolve_with_refresh()` uses `REFRESH_POLICY.early_refresh` (default 5 min) to refresh **before** expiry, not after.
- `CredentialContext` carries optional `callback_url`, `app_url`, `session_id` (private, with builder + accessors) for interactive OAuth2/SAML flows.
- `SecretString` serializes as `"[REDACTED]"` — tests must construct raw JSON for round-trip.
- `PendingToken`: 32-byte CSPRNG, URL-safe base64 no-pad, `Display` redacts. `generate()` is `pub(crate)` — used by `PendingStateStore` impls.
- `PendingStateStore` trait: 4-dimensional token binding (credential_kind, owner_id, session_id, token_id). Separate from `CredentialStore` — different lifecycle (minutes vs years), TTL-enforced, single-use consume.

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- Storage providers feature-gated: `storage-local`, `-aws`, `-postgres`, `-vault`, `-k8s`.
- v2 `LocalFileStore` (`store_local.rs`): filesystem `CredentialStore` impl behind `storage-local`. Uses `StoredFile` serde wrapper (base64 `data` field). Atomic write via temp-file rename. Path traversal validation on all ID inputs.

## Relations
- Depends on: nebula-core, nebula-eventbus. Peer: nebula-resource.

- `CredentialError` v2 variants: `InvalidInput`, `RefreshFailed` (with `RefreshErrorKind` + `RetryAdvice`), `RevokeFailed`, `CompositionNotAvailable`, `CompositionFailed`, `SchemeMismatch`. Supporting enums: `RefreshErrorKind`, `RetryAdvice`, `ResolutionStage` — all `#[non_exhaustive]`.

- v2 built-in credentials have enriched `parameters()` with descriptions, placeholders, and defaults from v1 protocol logic. `DatabaseCredential`: port defaults to 5432, `ssl_mode` defaults to "prefer" and is parsed into `SslMode` enum. `ApiKeyCredential`: optional `server` URL field. `SslMode` re-exported from crate root. `HeaderAuthCredential`: static credential producing `HeaderAuth` scheme from `header_name` + `header_value` parameters (migrated from v1 `HeaderAuthProtocol`).
- Scheme coercion impls in `scheme/coercion.rs`: `From<OAuth2Token> for BearerToken` (infallible), `TryFrom<ApiKeyAuth> for BearerToken` (Authorization header only, case-insensitive), `TryFrom<SamlAuth> for BearerToken` (requires assertion_b64). Errors use `CredentialError::SchemeMismatch`.
- `executor.rs`: framework-level `execute_resolve`/`execute_continue` free functions. 30s hard timeout via `tokio::time::timeout`. Handles `PendingState` lifecycle (put/consume via `PendingStateStore`). Returns `ResolveResponse<S>` (Complete/Pending/Retry). `ExecutorError` wraps timeout, credential, and store errors.
- `OAuth2Credential`: v2 impl with 3 grant types. `OAuth2State` (v2) includes `client_id`/`client_secret`/`token_url` for self-contained refresh. Re-exported as `OAuth2StateV2` (v1 `OAuth2State` still exists in protocols). `OAuth2Pending` zeroizes `client_secret`+`device_code`. Config extracted from `ParameterValues` (Approach B: no instance config). HTTP helpers in `credentials/oauth2_flow.rs`.

<!-- reviewed: 2026-03-30 — scheme coercion module added -->
<!-- reviewed: 2026-03-30 — 13 AuthScheme types complete: added HeaderAuth, HmacSecret, SshAuth, CertificateAuth, AwsAuth, LdapAuth, SamlAuth, KerberosAuth -->
<!-- updated: 2026-03-25 — polish v2 module names, rename types -->
<!-- reviewed: 2026-03-30 — absorbed auth RFCs into plans/, auth crate deleted -->
<!-- reviewed: 2026-03-30 — rotation framework verified v2-compatible: all 13 modules use only crate::core types (CredentialId, CredentialMetadata, CredentialState v2), no v1 manager imports -->
