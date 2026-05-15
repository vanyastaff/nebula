//! User profile endpoint handlers (global, no tenant scope).
//! Auth required but no org/workspace context needed.
//!
//! Every handler in this module is currently a 501-equivalent stub (audit
//! class (c)); the OpenAPI annotations describe the **planned** body shape
//! per ADR-0047 Stub Endpoint Policy. Tag suffix `(planned)` flags the
//! group in Swagger UI; once the underlying Plane-A backend extension lands
//! the only diff is removing `deprecated = true` and the 501 response.

use axum::{
    Extension, Json,
    extract::{Path, State},
};

use crate::{
    error::{ApiError, ApiResult, ProblemDetails},
    middleware::auth::AuthContext,
    models::{
        AckResponse, CreateTokenRequest, CreateTokenResponse, MeResponse, MyOrgsResponse,
        MyTokensResponse, UpdateMeRequest,
    },
    state::AppState,
};

/// `GET /api/v1/me` — current user profile.
///
/// Returns 501 today; payload schema is the planned shape once the
/// underlying Plane-A extension milestone closes.
#[utoipa::path(
    get,
    path = "/me",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 501, description = "Not yet implemented; tracked under Plane-A `me` extension milestone.", body = MeResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once Plane-A `me` extension milestone closes.")]
pub async fn get_me(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `PATCH /api/v1/me` — partial update of the current user's profile.
///
/// Returns 501 today; payload schema is the planned shape once the
/// underlying Plane-A extension milestone closes.
#[utoipa::path(
    patch,
    path = "/me",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    request_body = UpdateMeRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under Plane-A `me` extension milestone.", body = MeResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once Plane-A `me` extension milestone closes.")]
pub async fn update_me(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `GET /api/v1/me/orgs` — organisations the authenticated user belongs to.
///
/// Returns 501 today; payload schema is the planned shape once the
/// underlying Plane-A extension milestone closes.
#[utoipa::path(
    get,
    path = "/me/orgs",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 501, description = "Not yet implemented; tracked under Plane-A `me` extension milestone.", body = MyOrgsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once Plane-A `me` extension milestone closes.")]
pub async fn list_my_orgs(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `GET /api/v1/me/tokens` — list the user's personal access tokens
/// (metadata only — never the secret values).
///
/// Returns 501 today; payload schema is the planned shape once the
/// underlying Plane-A extension milestone closes.
#[utoipa::path(
    get,
    path = "/me/tokens",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    responses(
        (status = 501, description = "Not yet implemented; tracked under Plane-A `me` extension milestone.", body = MyTokensResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once Plane-A `me` extension milestone closes.")]
pub async fn list_my_tokens(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `POST /api/v1/me/tokens` — create a new personal access token.
///
/// Returns the token value **once** in the response body. Returns 501
/// today; payload schema is the planned shape once the underlying
/// Plane-A extension milestone closes.
#[utoipa::path(
    post,
    path = "/me/tokens",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    request_body = CreateTokenRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under Plane-A `me` extension milestone.", body = CreateTokenResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once Plane-A `me` extension milestone closes.")]
pub async fn create_token(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `DELETE /api/v1/me/tokens/{pat}` — revoke a personal access token.
///
/// Returns 501 today; payload schema is the planned shape once the
/// underlying Plane-A extension milestone closes.
#[utoipa::path(
    delete,
    path = "/me/tokens/{pat}",
    tag = "me (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("pat" = String, Path, description = "Personal access token identifier (`pat_<ULID>`)."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under Plane-A `me` extension milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Token does not exist.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once Plane-A `me` extension milestone closes.")]
pub async fn delete_token(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
    Path(_pat_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}
