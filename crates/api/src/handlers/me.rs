//! User profile endpoint handlers (global, no tenant scope).
//! Auth required but no org/workspace context needed.

use axum::{
    Extension, Json,
    extract::{Path, State},
};

use crate::{
    errors::{ApiError, ApiResult},
    middleware::auth::AuthContext,
    state::AppState,
};

/// GET /api/v1/me
pub async fn get_me(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Return current user profile
    Err(ApiError::Internal("not implemented".to_string()))
}

/// PATCH /api/v1/me
pub async fn update_me(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Update user profile fields
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/me/orgs
pub async fn list_my_orgs(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: List organizations the authenticated user belongs to
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/me/tokens
pub async fn list_my_tokens(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: List user's personal access tokens (metadata only, not secret values)
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/me/tokens
pub async fn create_token(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Create new PAT, return token value (shown only once)
    Err(ApiError::Internal("not implemented".to_string()))
}

/// DELETE /api/v1/me/tokens/{pat}
pub async fn delete_token(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthContext>,
    Path(_pat_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Revoke personal access token
    Err(ApiError::Internal("not implemented".to_string()))
}
