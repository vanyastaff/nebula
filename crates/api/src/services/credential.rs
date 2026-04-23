//! Credential service layer — business logic for credential operations.
//!
//! Each function takes an `AppState` reference plus domain-specific parameters
//! and returns `ApiResult`. All bodies currently return
//! `ApiError::ServiceUnavailable` stubs; they will be filled in once the
//! credential storage ports are wired into `AppState`.

use crate::{
    errors::{ApiError, ApiResult},
    models::credential::{
        ContinueResolveRequest, ContinueResolveResponse, CreateCredentialRequest,
        CredentialResponse, CredentialTypeInfo, ListCredentialTypesResponse, ListCredentialsQuery,
        ListCredentialsResponse, RefreshCredentialResponse, ResolveCredentialRequest,
        ResolveCredentialResponse, RevokeCredentialResponse, TestCredentialResponse,
        UpdateCredentialRequest,
    },
    state::AppState,
};

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Create a new credential in the given workspace.
///
/// # TODO
/// 1. Validate `req.data` against the credential type's `ValidSchema`.
/// 2. Encrypt sensitive fields via `EncryptionLayer`.
/// 3. Persist via `CredentialStore::put(mode=CreateOnly)`.
/// 4. Map `StoredCredential` → `CredentialResponse`.
pub async fn create_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _req: CreateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential storage integration pending — requires CredentialStore::put(CreateOnly) in AppState".into(),
    ))
}

/// Retrieve a single credential by ID within a workspace.
///
/// # TODO
/// 1. Fetch from `CredentialStore::get(cred)`.
/// 2. Verify caller has read access (owner_scope / tenant).
/// 3. Map `StoredCredential` → `CredentialResponse` (never include secrets).
pub async fn get_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<CredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential storage integration pending — requires CredentialStore::get() in AppState"
            .into(),
    ))
}

/// Update an existing credential in the workspace.
///
/// # TODO
/// 1. Fetch existing credential from `CredentialStore::get(cred)`.
/// 2. Merge provided fields into existing metadata.
/// 3. Re-validate and re-encrypt if `data` changed.
/// 4. Persist via `CredentialStore::put(mode=CompareAndSwap)` if version provided.
/// 5. Map updated `StoredCredential` → `CredentialResponse`.
pub async fn update_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
    _req: UpdateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential storage integration pending — requires CredentialStore::put(CompareAndSwap) in AppState".into(),
    ))
}

/// Delete a credential from the workspace.
///
/// # TODO
/// 1. Verify credential exists.
/// 2. Check no active workflows reference this credential.
/// 3. Delete via `CredentialStore::delete(cred)`.
/// 4. Emit audit log event.
pub async fn delete_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<()> {
    Err(ApiError::ServiceUnavailable(
        "credential storage integration pending — requires CredentialStore::delete() in AppState"
            .into(),
    ))
}

/// List credentials in the workspace with optional filters.
///
/// # TODO
/// 1. Query `CredentialStore::list()` with filters from `query`.
/// 2. Filter by user access scope (owner_scope / tenant).
/// 3. Map `Vec<StoredCredential>` → `Vec<CredentialSummary>`.
pub async fn list_credentials(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _query: ListCredentialsQuery,
) -> ApiResult<ListCredentialsResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential storage integration pending — requires CredentialStore::list() in AppState"
            .into(),
    ))
}

// ── Lifecycle ───────────────────────────────────────────────────────────────

/// Test credential connectivity against the external system.
///
/// # TODO
/// 1. Fetch `StoredCredential` and look up credential type.
/// 2. Check `TESTABLE` capability flag.
/// 3. Deserialize state, call `Credential::test()`.
/// 4. Map `TestResult` → `TestCredentialResponse`.
pub async fn test_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<TestCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential engine integration pending — requires credential type registry and Credential::test() dispatch".into(),
    ))
}

/// Force a token refresh for the credential.
///
/// # TODO
/// 1. Fetch `StoredCredential` and look up credential type.
/// 2. Check `REFRESHABLE` capability flag.
/// 3. Deserialize state, call `Credential::refresh()`.
/// 4. Persist updated state if refresh produced new tokens.
/// 5. Map `RefreshOutcome` → `RefreshCredentialResponse`.
pub async fn refresh_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<RefreshCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential engine integration pending — requires credential type registry and Credential::refresh() dispatch".into(),
    ))
}

/// Explicitly revoke the credential at the provider.
///
/// # TODO
/// 1. Fetch `StoredCredential` and look up credential type.
/// 2. Check `REVOCABLE` capability flag.
/// 3. Deserialize state, call `Credential::revoke()`.
/// 4. Optionally mark credential as revoked in store metadata.
/// 5. Map result → `RevokeCredentialResponse`.
pub async fn revoke_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _cred: &str,
) -> ApiResult<RevokeCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential engine integration pending — requires credential type registry and Credential::revoke() dispatch".into(),
    ))
}

// ── Acquisition ─────────────────────────────────────────────────────────────

/// Start credential acquisition / resolution.
///
/// # TODO
/// 1. Look up credential type by `req.credential_key`.
/// 2. Validate `req.data` against `Credential::schema()`.
/// 3. Call `Credential::resolve()`.
/// 4. Match on `ResolveResult`: Complete → persist, Pending → store pending state.
pub async fn resolve_credential(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _req: ResolveCredentialRequest,
) -> ApiResult<ResolveCredentialResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential resolve integration pending — requires CredentialTypeRegistry and PendingStateStore in AppState".into(),
    ))
}

/// Continue a multi-step credential acquisition.
///
/// # TODO
/// 1. Consume pending state from `PendingStateStore` using `req.pending_token`.
/// 2. Verify authenticated user matches the initiator.
/// 3. Call `Credential::continue_resolve()`.
/// 4. Match on `ResolveResult`: Complete → persist, Pending → store new pending.
pub async fn continue_resolve(
    _state: &AppState,
    _org: &str,
    _ws: &str,
    _req: ContinueResolveRequest,
) -> ApiResult<ContinueResolveResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential continue_resolve integration pending — requires PendingStateStore consumption in AppState".into(),
    ))
}

// ── Type discovery ──────────────────────────────────────────────────────────

/// List all registered credential types with their schemas and capabilities.
///
/// # TODO
/// 1. Iterate over `CredentialTypeRegistry` entries.
/// 2. For each type: call `Credential::metadata()`, `Credential::schema()`.
/// 3. Map to `Vec<CredentialTypeInfo>`, sort by key.
pub async fn list_credential_types(_state: &AppState) -> ApiResult<ListCredentialTypesResponse> {
    Err(ApiError::ServiceUnavailable(
        "credential type registry integration pending — requires CredentialTypeRegistry in AppState".into(),
    ))
}

/// Get metadata and schema for a specific credential type by key.
///
/// # TODO
/// 1. Look up credential type by `key` from `CredentialTypeRegistry`.
/// 2. Return `ApiError::NotFound` if no type registered with this key.
/// 3. Map to `CredentialTypeInfo`.
pub async fn get_credential_type(_state: &AppState, _key: &str) -> ApiResult<CredentialTypeInfo> {
    Err(ApiError::ServiceUnavailable(
        "credential type registry integration pending — requires CredentialTypeRegistry lookup by key".into(),
    ))
}
