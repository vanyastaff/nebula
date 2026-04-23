//! Resource listing endpoint handler (tenant-scoped).

use axum::{Extension, Json, extract::State};
use nebula_core::TenantContext;

use crate::{
    errors::{ApiError, ApiResult},
    state::AppState,
};

/// GET /api/v1/orgs/{org}/workspaces/{ws}/resources
pub async fn list_resources(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: List resources in workspace
    Err(ApiError::Internal("not implemented".to_string()))
}
