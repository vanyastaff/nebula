//! Credential handlers — workspace-scoped CRUD, lifecycle, acquisition,
//! type discovery, and OAuth2 transport.
//!
//! Thin HTTP handlers that validate inputs, delegate to the credential
//! service layer ([`crate::services::credential`]) or OAuth infrastructure
//! ([`crate::services::oauth`]), and return responses.

use axum::{
    Extension, Form, Json,
    extract::{Path, Query, State},
};

// Re-export the request/response types used by route wiring.
pub use crate::handlers::credential_oauth::{
    AuthorizationUriResponse as AuthUriResponse, OAuthCallbackResponse as CallbackResponse,
};
use crate::{
    errors::{ApiError, ApiResult},
    extractors::credential::{
        validate_credential_id, validate_credential_key, validate_credential_name,
        validate_data_is_object,
    },
    handlers::credential_oauth::{
        self as oauth_controller, AuthorizationUriResponse, OAuthCallbackBody, OAuthCallbackQuery,
        OAuthCallbackResponse,
    },
    middleware::auth::AuthenticatedUser,
    models::credential::{
        ContinueResolveRequest, ContinueResolveResponse, CreateCredentialRequest,
        CredentialResponse, CredentialTypeInfo, ListCredentialTypesResponse, ListCredentialsQuery,
        ListCredentialsResponse, RefreshCredentialResponse, ResolveCredentialRequest,
        ResolveCredentialResponse, RevokeCredentialResponse, TestCredentialResponse,
        UpdateCredentialRequest,
    },
    state::AppState,
};

// --- CRUD handlers ---

/// GET /orgs/{org}/workspaces/{ws}/credentials — List credentials with optional filters.
///
/// Returns paginated credential metadata. Never includes secret material.
pub async fn list_credentials(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws)): Path<(String, String)>,
    Query(query): Query<ListCredentialsQuery>,
) -> ApiResult<Json<ListCredentialsResponse>> {
    let response = crate::services::credential::list_credentials(&state, &org, &ws, query).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials — Create a new credential.
///
/// Validates the request body, then delegates to the credential store
/// for persistence. The `data` field is validated against the credential
/// type's schema before encryption and storage.
pub async fn create_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws)): Path<(String, String)>,
    Json(body): Json<CreateCredentialRequest>,
) -> ApiResult<Json<CredentialResponse>> {
    // --- Input validation ---
    validate_credential_key(&body.credential_key)?;
    let _name = validate_credential_name(&body.name)?;
    validate_data_is_object(&body.data)?;

    let response = crate::services::credential::create_credential(&state, &org, &ws, body).await?;
    Ok(Json(response))
}

/// GET /orgs/{org}/workspaces/{ws}/credentials/{cred} — Retrieve a single credential by ID.
///
/// Returns full credential metadata (never secret material).
pub async fn get_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<CredentialResponse>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    let response = crate::services::credential::get_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

/// PUT /orgs/{org}/workspaces/{ws}/credentials/{cred} — Update an existing credential.
///
/// Accepts partial updates — at least one field must be provided.
/// Supports optimistic concurrency via an optional `version` field.
pub async fn update_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
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

    let response =
        crate::services::credential::update_credential(&state, &org, &ws, &cred, body).await?;
    Ok(Json(response))
}

/// DELETE /orgs/{org}/workspaces/{ws}/credentials/{cred} — Delete a credential.
///
/// Permanently removes the credential and its encrypted state.
/// Returns a JSON acknowledgement on success.
pub async fn delete_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    // Validate path parameter.
    validate_credential_id(&cred)?;

    crate::services::credential::delete_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// --- Lifecycle handlers ---

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/test — Test credential connectivity.
///
/// Delegates to `Credential::test()` to verify the credential can
/// successfully authenticate against the external system. Returns 400
/// if the credential type does not support testing (`TESTABLE = false`).
pub async fn test_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<TestCredentialResponse>> {
    validate_credential_id(&cred)?;

    let response = crate::services::credential::test_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh — Force token refresh.
///
/// Delegates to `Credential::refresh()` to force a token refresh
/// (e.g. OAuth2 `refresh_token` grant). Returns 400 if the credential
/// type does not support refreshing (`REFRESHABLE = false`).
pub async fn refresh_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<RefreshCredentialResponse>> {
    validate_credential_id(&cred)?;

    let response =
        crate::services::credential::refresh_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke — Explicitly revoke credential at
/// provider.
///
/// Delegates to `Credential::revoke()` to explicitly revoke the
/// credential at the provider. Returns 400 if the credential type
/// does not support revocation (`REVOCABLE = false`).
pub async fn revoke_credential(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthenticatedUser>,
    Path((org, ws, cred)): Path<(String, String, String)>,
) -> ApiResult<Json<RevokeCredentialResponse>> {
    validate_credential_id(&cred)?;

    let response = crate::services::credential::revoke_credential(&state, &org, &ws, &cred).await?;
    Ok(Json(response))
}

// --- Acquisition handlers ---

/// POST /orgs/{org}/workspaces/{ws}/credentials/resolve — Start credential acquisition.
///
/// Accepts the credential type key and form field values, dispatches to
/// the appropriate `Credential::resolve()` implementation. Returns either:
/// - `Complete { credential_id }` for static credentials (api_key, basic_auth, client_credentials)
/// - `Pending { pending_token, interaction }` for interactive flows (OAuth auth_code, device_code)
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
        crate::services::credential::resolve_credential(&state, &org, &ws, request).await?;
    Ok(Json(response))
}

/// POST /orgs/{org}/workspaces/{ws}/credentials/resolve/continue — Continue a multi-step
/// acquisition.
///
/// Accepts a pending token from a previous `Pending` response and the user's
/// input (authorization code, device confirmation, challenge answer, etc.).
/// Returns the same response shape as `resolve_credential`.
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
        crate::services::credential::continue_resolve(&state, &org, &ws, request).await?;
    Ok(Json(response))
}

// --- Type discovery handlers ---

/// GET /credentials/types — List all registered credential types.
///
/// Returns metadata, capability flags, and JSON Schema for each registered
/// credential type. This is a read-only discovery endpoint — no credentials
/// are created or modified.
pub async fn list_credential_types(
    State(state): State<AppState>,
) -> ApiResult<Json<ListCredentialTypesResponse>> {
    let response = crate::services::credential::list_credential_types(&state).await?;
    Ok(Json(response))
}

/// GET /credentials/types/{key} — Get metadata and schema for a credential type.
///
/// Returns detailed information including the JSON Schema that describes
/// the input fields required to create a credential of this type.
pub async fn get_credential_type(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<Json<CredentialTypeInfo>> {
    // ── Input validation ────────────────────────────────────────────
    validate_credential_key(&key)?;

    let response = crate::services::credential::get_credential_type(&state, &key).await?;
    Ok(Json(response))
}

// --- OAuth2 handlers ---

/// GET /credentials/{id}/oauth2/auth — Generate OAuth2 authorization URL.
///
/// Builds the provider authorization URL with PKCE challenge and signed
/// state parameter. The frontend redirects the user to this URL to begin
/// the OAuth2 authorization code flow.
pub async fn get_oauth2_authorize_url(
    path: Path<String>,
    state: State<AppState>,
    user: Extension<AuthenticatedUser>,
    query: Query<crate::services::oauth::flow::AuthorizationUriRequest>,
) -> ApiResult<Json<AuthorizationUriResponse>> {
    oauth_controller::get_oauth2_authorize_url(path, state, user, query).await
}

/// GET /credentials/{id}/oauth2/callback — Handle OAuth2 callback (query params).
///
/// Receives the authorization code and signed state via query parameters,
/// exchanges the code for tokens, and persists the OAuth2 credential state.
pub async fn get_oauth2_callback(
    path: Path<String>,
    state: State<AppState>,
    user: Extension<AuthenticatedUser>,
    query: Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    oauth_controller::get_oauth2_callback(path, state, user, query).await
}

/// POST /credentials/{id}/oauth2/callback — Handle OAuth2 callback (form_post).
///
/// Accepts `application/x-www-form-urlencoded` bodies for providers that use
/// the `form_post` response mode.
pub async fn post_oauth2_callback(
    path: Path<String>,
    state: State<AppState>,
    user: Extension<AuthenticatedUser>,
    body: Form<OAuthCallbackBody>,
) -> ApiResult<Json<OAuthCallbackResponse>> {
    oauth_controller::post_oauth2_callback(path, state, user, body).await
}
