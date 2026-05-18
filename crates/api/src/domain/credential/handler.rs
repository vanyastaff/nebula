//! Credential handlers — workspace-scoped CRUD, lifecycle, acquisition,
//! type discovery, and OAuth2 transport.
//!
//! Thin HTTP handlers that validate inputs, delegate to the credential
//! service layer ([`crate::transport::credential`]) or OAuth infrastructure
//! ([`crate::transport::oauth`]), and return responses.

use axum::{
    Extension, Form, Json,
    extract::{Path, Query, State},
};
use nebula_core::TenantContext;

// Re-export the request/response types used by route wiring.
pub use super::oauth::{
    AuthorizationUriResponse as AuthUriResponse, OAuthCallbackResponse as CallbackResponse,
};
use super::{
    dto::{
        ContinueResolveRequest, ContinueResolveResponse, CreateCredentialRequest,
        CredentialResponse, CredentialTypeInfo, ListCredentialTypesResponse, ListCredentialsQuery,
        ListCredentialsResponse, RefreshCredentialResponse, ResolveCredentialRequest,
        ResolveCredentialResponse, RevokeCredentialResponse, TestCredentialResponse,
        UpdateCredentialRequest,
    },
    oauth::{
        self as oauth_controller, AuthorizationUriResponse, OAuthCallbackBody, OAuthCallbackQuery,
        OAuthCallbackResponse,
    },
};
use crate::{
    domain::shared::AckResponse,
    error::{ApiError, ApiResult, ProblemDetails},
    extractors::credential::{
        validate_credential_id, validate_credential_key, validate_credential_name,
        validate_data_is_object,
    },
    middleware::auth::AuthenticatedUser,
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
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ListCredentialsQuery,
    ),
    responses(
        (status = 200, description = "Page of credential summaries (no secret material).", body = ListCredentialsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
    ),
)]
pub async fn list_credentials(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Query(query): Query<ListCredentialsQuery>,
) -> ApiResult<Json<ListCredentialsResponse>> {
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    let response = crate::transport::credential::list_credentials(&state, &owner_id, query).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials — Create a new credential.
///
/// Validates the request body, then delegates to the credential store
/// for persistence. When a credential-schema port is configured
/// (credential-schema validation), `data` is validated against the credential type's
/// resolved schema before encryption and storage; if no validator is
/// configured the request is rejected with 503 — `data` is never
/// persisted unvalidated.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
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
        (status = 503, description = "Credential-schema port not configured: `data` cannot be validated, so it is not persisted (credential-schema validation, honest capability fail-closed).", body = ProblemDetails),
    ),
)]
pub async fn create_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(body): Json<CreateCredentialRequest>,
) -> ApiResult<Json<CredentialResponse>> {
    // --- Input validation ---
    validate_credential_key(&body.credential_key)?;
    let _name = validate_credential_name(&body.name)?;
    validate_data_is_object(&body.data)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    let response = crate::transport::credential::create_credential(&state, &owner_id, body).await?;
    Ok(Json(response))
}

/// GET /orgs/{org}/workspaces/{ws}/credentials/{cred} — Retrieve a single credential by ID.
///
/// Returns full credential metadata (never secret material).
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
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
    ),
)]
pub async fn get_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<CredentialResponse>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    let response = crate::transport::credential::get_credential(&state, &owner_id, &cred).await?;
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
    security(("bearer" = []), ("api_key" = [])),
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
        (status = 409, description = "Optimistic-concurrency version mismatch.", body = ProblemDetails),
        (status = 503, description = "Credential-schema port not configured: supplied `data` cannot be validated, so it is not persisted (credential-schema validation, honest capability fail-closed).", body = ProblemDetails),
    ),
)]
pub async fn update_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
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
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    let response =
        crate::transport::credential::update_credential(&state, &owner_id, &cred, body).await?;
    Ok(Json(response))
}

/// DELETE /orgs/{org}/workspaces/{ws}/credentials/{cred} — Delete a credential.
///
/// Permanently removes the credential and its encrypted state.
/// Returns a JSON acknowledgement on success.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Credential deleted.", body = AckResponse),
        (status = 400, description = "Invalid credential identifier.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
    ),
)]
pub async fn delete_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<AckResponse>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    crate::transport::credential::delete_credential(&state, &owner_id, &cred).await?;
    Ok(Json(AckResponse::ok()))
}

// --- Lifecycle handlers ---

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/test — Test credential connectivity.
///
/// Delegates to `Credential::test()` to verify the credential can
/// successfully authenticate against the external system. Returns 400
/// if the credential type does not support testing (`TESTABLE = false`).
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/test",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Connectivity test result (success flag + message).", body = TestCredentialResponse),
        (status = 400, description = "Invalid credential identifier or credential type does not support testing.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Credential does not exist.", body = ProblemDetails),
    ),
)]
pub async fn test_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<TestCredentialResponse>> {
    validate_credential_id(&cred)?;

    let response = crate::transport::credential::test_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh — Force token refresh.
///
/// Delegates to `Credential::refresh()` to force a token refresh
/// (e.g. OAuth2 `refresh_token` grant). Returns 400 if the credential
/// type does not support refreshing (`REFRESHABLE = false`).
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
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
    ),
)]
pub async fn refresh_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<RefreshCredentialResponse>> {
    validate_credential_id(&cred)?;

    let response =
        crate::transport::credential::refresh_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke — Explicitly revoke credential at
/// provider.
///
/// Delegates to `Credential::revoke()` to explicitly revoke the
/// credential at the provider. Returns 400 if the credential type
/// does not support revocation (`REVOCABLE = false`).
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
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
    ),
)]
pub async fn revoke_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<RevokeCredentialResponse>> {
    validate_credential_id(&cred)?;

    let response =
        crate::transport::credential::revoke_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

// --- Acquisition handlers ---

/// POST /orgs/{org}/workspaces/{ws}/credentials/resolve — Start credential acquisition.
///
/// Accepts the credential type key and form field values, dispatches to
/// the appropriate `Credential::resolve()` implementation. Returns either:
/// - `Complete { credential_id }` for static credentials (api_key, basic_auth, client_credentials)
/// - `Pending { pending_token, interaction }` for interactive flows (OAuth auth_code, device_code)
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/resolve",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
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
        (status = 503, description = "Honest stub (honest capability contract): the route is reachable but generic credential resolution is engine-owned (`Credential::resolve` dispatch via `nebula-engine::credential`) and requires a `CredentialRegistry` not wired into this build, so it refuses rather than faking success.", body = ProblemDetails),
    ),
)]
pub async fn resolve_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws)): Path<(String, String)>,
    Json(request): Json<ResolveCredentialRequest>,
) -> ApiResult<Json<ResolveCredentialResponse>> {
    // ── Input validation ────────────────────────────────────────────
    validate_credential_key(&request.credential_key)?;
    validate_data_is_object(&request.data)?;

    let response =
        crate::transport::credential::resolve_credential(&state, &org, &ws, request).await?;
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
    security(("bearer" = []), ("api_key" = [])),
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
        (status = 503, description = "Honest stub (honest capability contract): the route is reachable but generic interactive continuation is engine-owned (`Interactive::continue_resolve` dispatch via `nebula-engine::credential`) and requires a `CredentialRegistry` not wired into this build, so it refuses rather than faking success.", body = ProblemDetails),
    ),
)]
pub async fn continue_resolve_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws)): Path<(String, String)>,
    Json(request): Json<ContinueResolveRequest>,
) -> ApiResult<Json<ContinueResolveResponse>> {
    // ── Input validation ────────────────────────────────────────────
    if request.pending_token.is_empty() {
        return Err(ApiError::Validation {
            detail: "pending_token must not be empty".to_string(),
            errors: vec![],
        });
    }

    let response =
        crate::transport::credential::continue_resolve(&state, &org, &ws, request).await?;
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
        (status = 503, description = "Credential-schema port not configured (credential-schema validation, honest capability stub).", body = ProblemDetails),
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
        ("key" = String, Path, description = "Credential type key (e.g. `oauth2`, `api_key`)."),
    ),
    responses(
        (status = 200, description = "Credential type metadata with input schema.", body = CredentialTypeInfo),
        (status = 400, description = "Invalid credential type key.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Credential type not registered.", body = ProblemDetails),
        (status = 503, description = "Credential-schema port not configured (credential-schema validation, honest capability stub).", body = ProblemDetails),
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

// --- OAuth2 handlers ---

/// GET /credentials/{id}/oauth2/auth — Generate OAuth2 authorization URL.
///
/// System-level OAuth credential routes are intentionally disabled. OAuth
/// state contains credential ownership and must be created through the
/// workspace-scoped route below.
#[utoipa::path(
    get,
    path = "/credentials/{id}/oauth2/auth",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("id" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Authorization URL plus signed opaque state.", body = AuthorizationUriResponse),
        (status = 400, description = "Invalid OAuth configuration (e.g. malformed authorization URL).", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
pub async fn get_oauth2_authorize_url(
    Path(_cred): Path<String>,
    State(_state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Query(_query): Query<crate::transport::oauth::flow::AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    Err(ApiError::Gone(
        "OAuth credential flow must use workspace-scoped routes".to_owned(),
    ))
}

/// GET /credentials/{id}/oauth2/callback — Handle OAuth2 callback (query params).
///
/// System-level callback routes are intentionally disabled. The signed
/// pending state is tenant-bound and must be consumed through the
/// workspace-scoped callback route below.
#[utoipa::path(
    get,
    path = "/credentials/{id}/oauth2/callback",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("id" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
        ("code" = String, Query, description = "Authorization code from the provider."),
        ("state" = String, Query, description = "Signed opaque state from `oauth2/auth`."),
    ),
    responses(
        (status = 200, description = "Tokens exchanged and persisted.", body = OAuthCallbackResponse),
        (status = 401, description = "State validation failed or pending state expired/already-consumed.", body = ProblemDetails),
    ),
)]
pub async fn get_oauth2_callback(
    Path(_cred): Path<String>,
    State(_state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Query(_query): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    Err(ApiError::Gone(
        "OAuth credential flow must use workspace-scoped routes".to_owned(),
    ))
}

/// POST /credentials/{id}/oauth2/callback — Handle OAuth2 callback (form_post).
///
/// System-level callback routes are intentionally disabled. The signed
/// pending state is tenant-bound and must be consumed through the
/// workspace-scoped callback route below.
#[utoipa::path(
    post,
    path = "/credentials/{id}/oauth2/callback",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("id" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    request_body(content = OAuthCallbackBody, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Tokens exchanged and persisted.", body = OAuthCallbackResponse),
        (status = 401, description = "State validation failed or pending state expired/already-consumed.", body = ProblemDetails),
    ),
)]
pub async fn post_oauth2_callback(
    Path(_cred): Path<String>,
    State(_state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Form(_body): Form<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    Err(ApiError::Gone(
        "OAuth credential flow must use workspace-scoped routes".to_owned(),
    ))
}

/// GET /orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/auth — Generate OAuth2 authorization URL.
///
/// Builds the provider authorization URL with PKCE challenge and tenant-bound
/// signed state. The frontend redirects the user to this URL to begin the
/// OAuth2 authorization code flow.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/auth",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Authorization URL plus signed opaque state.", body = AuthorizationUriResponse),
        (status = 400, description = "Invalid OAuth configuration (e.g. malformed authorization URL or unsafe token endpoint).", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
    ),
)]
pub async fn get_oauth2_authorize_url_scoped(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
    Query(query): Query<crate::transport::oauth::flow::AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    validate_credential_id(&cred)?;
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    oauth_controller::get_oauth2_authorize_url_for_owner(
        &cred,
        &state,
        &user,
        query,
        Some(owner_id),
    )
    .await
}

/// GET /orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/callback — Handle OAuth2 callback.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/callback",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
        ("code" = String, Query, description = "Authorization code from the provider."),
        ("state" = String, Query, description = "Signed opaque state from `oauth2/auth`."),
    ),
    responses(
        (status = 200, description = "Tokens exchanged and persisted.", body = OAuthCallbackResponse),
        (status = 401, description = "State validation failed, tenant mismatch, or pending state expired/already-consumed.", body = ProblemDetails),
    ),
)]
pub async fn get_oauth2_callback_scoped(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
    Query(query): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    validate_credential_id(&cred)?;
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    oauth_controller::handle_callback_for_owner(
        &cred,
        &state,
        &user,
        query.code,
        query.state,
        owner_id,
        |req| async move { crate::transport::oauth::flow::exchange_code(&req).await },
    )
    .await
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/callback — Handle OAuth2 form_post callback.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/credentials/{cred}/oauth2/callback",
    tag = "workspaces.credentials",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("cred" = String, Path, description = "Credential identifier (`cred_<ULID>`)."),
    ),
    request_body(content = OAuthCallbackBody, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Tokens exchanged and persisted.", body = OAuthCallbackResponse),
        (status = 401, description = "State validation failed, tenant mismatch, or pending state expired/already-consumed.", body = ProblemDetails),
    ),
)]
pub async fn post_oauth2_callback_scoped(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, cred)): Path<(String, String, String)>,
    Form(body): Form<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    validate_credential_id(&cred)?;
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;
    let owner_id = crate::transport::credential::owner_id_from_scope(&scope);
    oauth_controller::handle_callback_for_owner(
        &cred,
        &state,
        &user,
        body.code,
        body.state,
        owner_id,
        |req| async move { crate::transport::oauth::flow::exchange_code(&req).await },
    )
    .await
}
