//! Resource listing endpoint handler (tenant-scoped).
//!
//! Currently a 501-equivalent stub (audit class (c)); the OpenAPI
//! annotation describes the **planned** body shape per ADR-0047 Stub
//! Endpoint Policy. Tag suffix `(planned)` flags the group in Swagger UI;
//! once the resource catalog backend lands the only diff is removing
//! `deprecated = true` and the 501 response.

use axum::{Extension, Json, extract::State};
use nebula_core::TenantContext;

use crate::{
    errors::{ApiError, ApiResult, ProblemDetails},
    models::ListResourcesResponse,
    state::AppState,
};

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` — list workspace resources.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources",
    tag = "workspaces.resources (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under resource catalog milestone.", body = ListResourcesResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once resource catalog milestone closes.")]
pub async fn list_resources(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::Internal("not implemented".to_string()))
}
