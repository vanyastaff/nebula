//! Resource listing endpoint handler (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` is a config-CRUD
//! read endpoint: it returns the persisted resource *definitions* for a
//! workspace. Resource lifecycle (acquire/refresh/revoke) is owned by the
//! engine and is intentionally NOT exposed over HTTP.
//!
//! The resource catalog backend is optional on [`AppState`]
//! (`resource_repo`); when it is not configured the endpoint reports
//! `503 Service Unavailable`, matching the action/plugin catalog
//! convention rather than the retired ADR-0047 501 stub.

use axum::{Extension, Json, extract::State};
use nebula_core::{ResourceId, TenantContext};

use crate::{
    errors::{ApiError, ApiResult, ProblemDetails},
    models::{ListResourcesResponse, ResourceSummary},
    state::AppState,
};

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` — list workspace resources.
///
/// Returns resource definitions scoped to the caller's workspace. Soft-deleted
/// rows are excluded. The raw `config` blob is never surfaced — only the
/// non-secret summary fields (ADR-0028 §7).
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    responses(
        (status = 200, description = "Resource definitions for the workspace (no raw config).", body = ListResourcesResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn list_resources(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<ListResourcesResponse>> {
    // Workspace presence is guaranteed by the tenancy middleware for this
    // route; the explicit check keeps the handler honest if the route is
    // ever remounted. RBAC parity with the other workspace-scoped list
    // endpoints (`list_workflows`, `list_executions`) — no per-handler
    // permission gate under the current shared-trust JWT model.
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    let entries = repo.list(workspace_id.as_bytes().as_slice()).await?;

    let resources = entries
        .into_iter()
        // Soft-deleted definitions are not part of the catalog view.
        .filter(|entry| entry.deleted_at.is_none())
        .map(|entry| {
            let id = nebula_storage::mapping::bytes_to_id(&entry.id)
                .map(|bytes| ResourceId::from_bytes(bytes).to_string())
                .map_err(ApiError::from)?;
            Ok(ResourceSummary {
                id,
                name: entry.display_name,
                kind: entry.kind,
                version: entry.version.to_string(),
                // Workflow attachment is not tracked by the resource store
                // yet; advertised honestly as empty rather than fabricated.
                attached_to_workflows: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(ListResourcesResponse { resources }))
}
