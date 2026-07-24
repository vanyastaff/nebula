//! Credential handlers — workspace-scoped CRUD, lifecycle, universal
//! acquisition, and type discovery.
//!
//! Thin HTTP handlers that validate inputs, delegate to the credential
//! service layer ([`crate::transport::credential`]), and return responses.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use nebula_core::TenantContext;

use super::dto::{
    ContinueResolveRequest, ContinueResolveResponse, CreateCredentialRequest, CredentialResponse,
    CredentialTypeInfo, ListCredentialTypesResponse, ListCredentialsQuery, ListCredentialsResponse,
    RefreshCredentialResponse, ResolveCredentialRequest, ResolveCredentialResponse,
    RevokeCredentialResponse, TestCredentialResponse, UpdateCredentialRequest,
};
use crate::{
    domain::shared::AckResponse,
    error::{ApiError, ApiResult, ProblemDetails},
    extractors::credential::{
        validate_credential_id, validate_credential_key, validate_credential_name,
        validate_data_is_object,
    },
    middleware::auth::AuthenticatedPrincipal,
    state::AppState,
};

// --- CRUD handlers ---

/// GET /orgs/{org}/workspaces/{ws}/credentials — List credentials with optional filters.
///
/// Returns paginated credential metadata. Never includes secret material.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/credentials",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ListCredentialsQuery,
    ),
    responses(
        (status = 200, description = "Page of credential summaries (no secret material).", body = ListCredentialsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 503, description = "Credential authority or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn list_credentials(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Query(query): Query<ListCredentialsQuery>,
) -> ApiResult<Json<ListCredentialsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::list_credentials(&state, &principal, &scope, query).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials — Create a new credential.
///
/// The authenticated command gateway authorizes first, then the
/// credential-owned controller/service validates the type-specific data and
/// persists it. The catalog/form read-model is not mutation authority.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    request_body = CreateCredentialRequest,
    responses(
        (status = 200, description = "Credential created.", body = CredentialResponse),
        (status = 400, description = "Validation error: `data` failed the credential type's schema, or key/name shape invalid.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 409, description = "Credential id/name is already reserved, or the persistence outcome is unknown and must be reconciled.", body = ProblemDetails),
        (status = 503, description = "Credential authority or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn create_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(body): Json<CreateCredentialRequest>,
) -> ApiResult<Json<CredentialResponse>> {
    // --- Input validation ---
    validate_credential_key(&body.credential_key)?;
    let _name = validate_credential_name(&body.name)?;
    validate_data_is_object(&body.data)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::create_credential(&state, &principal, &scope, body).await?;
    Ok(Json(response))
}

/// GET /orgs/{org}/workspaces/{ws}/credentials/{cred} — Retrieve a single credential by ID.
///
/// Returns full credential metadata (never secret material).
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Credential metadata (no secret material).", body = CredentialResponse),
        (status = 400, description = "Invalid credential identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
        (status = 503, description = "Credential authority or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn get_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<CredentialResponse>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::get_credential(&state, &principal, &scope, &cred).await?;
    Ok(Json(response))
}

/// PUT /orgs/{org}/workspaces/{ws}/credentials/{cred} — Update an existing credential.
///
/// Accepts partial updates — at least one field must be provided.
/// Supports optimistic concurrency via an optional `version` field.
#[utoipa::path(
    put,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    request_body = UpdateCredentialRequest,
    responses(
        (status = 200, description = "Credential updated.", body = CredentialResponse),
        (status = 400, description = "Validation error: supplied `data` failed the credential type's schema, or no fields provided.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
        (status = 409, description = "Version/name conflict, exhausted version, or unknown persistence outcome requiring reconciliation.", body = ProblemDetails),
        (status = 503, description = "Credential authority or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn update_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
    Json(body): Json<UpdateCredentialRequest>,
) -> ApiResult<Json<CredentialResponse>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    // At least one field must be provided for update.
    let has_updates = body.name.is_some()
        || body.description.is_some()
        || body.data.is_some()
        || body.tags.is_some();
    if !has_updates {
        return Err(ApiError::Validation {
            detail: "At least one field must be provided for update".to_string(),
            errors: vec![],
        });
    }

    // Validate name if provided.
    if let Some(ref name) = body.name {
        validate_credential_name(name)?;
    }

    // Validate data shape if provided.
    if let Some(ref data) = body.data {
        validate_data_is_object(data)?;
    }

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::update_credential(&state, &principal, &scope, &cred, body)
            .await?;
    Ok(Json(response))
}

/// DELETE /orgs/{org}/workspaces/{ws}/credentials/{cred} — Delete a credential.
///
/// Replaces the live row with a secret-free tombstone.
///
/// The identifier stays reserved and ordinary management reads treat it as
/// absent. This endpoint does not claim physical erasure from WAL, snapshots,
/// or backups.
/// Returns a JSON acknowledgement on success.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Credential tombstoned; live material is no longer available.", body = AckResponse),
        (status = 400, description = "Invalid credential identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent/version conflict or unknown persistence outcome requiring reconciliation.", body = ProblemDetails),
        (status = 503, description = "Credential authority or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn delete_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<AckResponse>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    crate::transport::credential::delete_credential(&state, &principal, &scope, &cred).await?;
    Ok(Json(AckResponse::ok()))
}

// --- Lifecycle handlers ---

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/test — Test credential connectivity.
///
/// Dispatches the registered `Testable::test` capability to verify the
/// credential against the external system. Returns 400 when the credential
/// type has no registered test capability.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/test",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Tagged connectivity test result with platform-owned messages and a required frozen v1 code on failure.", body = TestCredentialResponse),
        (status = 400, description = "Invalid credential identifier or credential type does not support testing.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
        (status = 503, description = "Credential authority, provider, or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn test_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<TestCredentialResponse>> {
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::test_credential(&state, &principal, &scope, &cred).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh — Force token refresh.
///
/// Delegates to `Credential::refresh()` to force a token refresh
/// (e.g. OAuth2 `refresh_token` grant). Returns 400 if the credential
/// type does not support refreshing (`REFRESHABLE = false`).
///
/// An exact pre-dispatch refusal or a complete provider response proving no
/// effect returns 409. When `Retry-After` is present, callers wait for that
/// delay before retrying. Without `Retry-After`, callers must not retry
/// automatically; they reconcile or reconnect when durable local finalization
/// failed.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Refresh result (success flag + new expiry, if changed).", body = RefreshCredentialResponse),
        (status = 400, description = "Invalid credential identifier or credential type does not support refreshing.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
        (status = 409, description = "The integration credential requires reconnection, refresh was proven not applied (Retry-After gives the earliest retry time), durable local refresh finalization definitely failed and requires reconciliation, a concurrent version changed, or a mutation acknowledgement was lost and state must be reconciled.", body = ProblemDetails, content_type = "application/problem+json", headers(
            ("Retry-After" = u64, description = "Optional non-zero whole-second delay before a proven no-effect refresh may be retried.")
        )),
        (status = 503, description = "Credential authority, pre-provider coordination, or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn refresh_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<RefreshCredentialResponse>> {
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::refresh_credential(&state, &principal, &scope, &cred).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke — Explicitly revoke credential at
/// provider.
///
/// Delegates to `Credential::revoke()` to explicitly revoke the
/// credential at the provider. Returns 400 if the credential type
/// does not support revocation (`REVOCABLE = false`).
///
/// When the revoke outcome is known but durable local finalization fails,
/// callers receive 409 and must reconcile credential state instead of
/// retrying automatically.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Revocation result.", body = RevokeCredentialResponse),
        (status = 400, description = "Invalid credential identifier or credential type does not support revocation.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
        (status = 409, description = "Concurrent version changed; the revoke outcome is known but durable local finalization failed (do not retry automatically; reconcile credential state); or a mutation acknowledgement was lost and credential state must be reconciled.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "Credential authority, pre-provider coordination, or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn revoke_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<RevokeCredentialResponse>> {
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::revoke_credential(&state, &principal, &scope, &cred).await?;
    Ok(Json(response))
}

// --- Acquisition handlers ---

/// POST /orgs/{org}/workspaces/{ws}/credentials/resolve — Start credential acquisition.
///
/// Accepts the credential type key and form field values, dispatches to
/// the appropriate `Credential::resolve()` implementation. Returns either:
/// - `Complete { credential_id }` for default static credentials (`api_key`, `basic_auth`,
///   `signing_key`)
/// - `Pending { pending_token, interaction }` for an explicitly composed interactive type (the
///   default registry currently contains none)
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/resolve",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    request_body = ResolveCredentialRequest,
    responses(
        (status = 200, description = "Resolution outcome — tagged enum: `complete` with `credential_id`, or `pending` with `pending_token` and the next `interaction` step.", body = ResolveCredentialResponse),
        (status = 400, description = "Validation error (key or data shape).", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 409, description = "Generated identity/name conflict or unknown persistence outcome requiring reconciliation.", body = ProblemDetails),
        (status = 503, description = "Credential authority, provider, or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn resolve_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(request): Json<ResolveCredentialRequest>,
) -> ApiResult<Json<ResolveCredentialResponse>> {
    // ── Input validation ────────────────────────────────────────────
    validate_credential_key(&request.credential_key)?;
    validate_data_is_object(&request.data)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::resolve_credential(&state, &principal, &scope, request)
            .await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/resolve/continue — Continue a multi-step
/// acquisition.
///
/// Accepts a pending token from a previous `Pending` response and the user's
/// input (authorization code, device confirmation, challenge answer, etc.).
/// Returns the same response shape as `resolve_credential`.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/resolve/continue",
    tag = "workspaces.credentials",
    security(("bearer" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    request_body = ContinueResolveRequest,
    responses(
        (status = 200, description = "Resolution outcome (same shape as `resolve`).", body = ResolveCredentialResponse),
        (status = 400, description = "Validation error (e.g. empty `pending_token`).", body = ProblemDetails),
        (status = 401, description = "Authentication required or pending token expired/already-consumed.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 409, description = "Generated identity/name conflict or unknown persistence outcome requiring reconciliation.", body = ProblemDetails),
        (status = 503, description = "Credential authority, provider, or persistence is temporarily unavailable.", body = ProblemDetails),
    ),
)]
pub async fn continue_resolve_credential(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthenticatedPrincipal>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(request): Json<ContinueResolveRequest>,
) -> ApiResult<Json<ContinueResolveResponse>> {
    // ── Input validation ────────────────────────────────────────────
    validate_credential_key(&request.credential_key)?;
    if request.pending_token.is_empty() {
        return Err(ApiError::Validation {
            detail: "pending_token must not be empty".to_string(),
            errors: vec![],
        });
    }

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let response =
        crate::transport::credential::continue_resolve(&state, &principal, &scope, request).await?;
    Ok(Json(response))
}

// --- Type discovery handlers ---

/// GET /credentials/types — List all registered credential types.
///
/// Returns metadata, capability flags, and JSON Schema for each registered
/// credential type. This is a read-only discovery endpoint — no credentials
/// are created or modified.
#[utoipa::path(
    get,
    path = "/credentials/types",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 200, description = "Registered credential types with capability flags and input schema.", body = ListCredentialTypesResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 503, description = "Credential catalog/form read-model is not configured.", body = ProblemDetails),
    ),
)]
pub async fn list_credential_types(
    State(state): State<AppState>,
) -> ApiResult<Json<ListCredentialTypesResponse>> {
    let response = crate::transport::credential::list_credential_types(&state).await?;
    Ok(Json(response))
}

/// GET /credentials/types/{key} — Get metadata and schema for a credential type.
///
/// Returns detailed information including the JSON Schema that describes
/// the input fields required to create a credential of this type.
#[utoipa::path(
    get,
    path = "/credentials/types/{key}",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("key" = String, Path, description = "Credential type key (e.g. `api_key`, `basic_auth`)."),
    ),
    responses(
        (status = 200, description = "Credential type metadata with input schema.", body = CredentialTypeInfo),
        (status = 400, description = "Invalid credential type key.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Credential type not registered.", body = ProblemDetails),
        (status = 503, description = "Credential catalog/form read-model is not configured.", body = ProblemDetails),
    ),
)]
pub async fn get_credential_type(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<CredentialTypeInfo>> {
    // ── Input validation ────────────────────────────────────────────
    validate_credential_key(&key)?;

    let response = crate::transport::credential::get_credential_type(&state, &key).await?;
    Ok(Json(response))
}
