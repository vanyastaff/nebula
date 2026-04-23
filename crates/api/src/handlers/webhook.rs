//! Webhook trigger endpoint handlers.
//! Special: no standard auth middleware — triggers have per-trigger auth.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::{
    errors::{ApiError, ApiResult},
    state::AppState,
};

/// POST /api/v1/hooks/{org}/{ws}/{trigger_slug}
pub async fn handle_webhook_post(
    State(_state): State<AppState>,
    Path((_org, _ws, _trigger_slug)): Path<(String, String, String)>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    // TODO: Validate per-trigger auth
    // TODO: Enqueue trigger event
    // TODO: Return 202 Accepted
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/hooks/{org}/{ws}/{trigger_slug}
pub async fn handle_webhook_get(
    State(_state): State<AppState>,
    Path((_org, _ws, _trigger_slug)): Path<(String, String, String)>,
) -> ApiResult<StatusCode> {
    // TODO: Some webhooks use GET (e.g., verification challenges)
    Err(ApiError::Internal("not implemented".to_string()))
}
