//! Resource catalog endpoint handlers (workspace-scoped).
//!
//! `GET /api/v1/orgs/{org}/workspaces/{ws}/resources` and
//! `GET .../resources/{res}` are config-CRUD read endpoints: they return
//! the persisted resource *definitions* for a workspace. Resource
//! lifecycle (acquire/refresh/revoke) is owned by the engine and is
//! intentionally NOT exposed over HTTP.
//!
//! The resource catalog backend is optional on [`AppState`]
//! (`resource_repo`); when it is not configured the endpoints report
//! `503 Service Unavailable`, matching the action/plugin catalog
//! convention rather than the retired ADR-0047 501 stub.
//!
//! Tenant isolation for the single-resource read: `ResourceRepo::get` is
//! looked up purely by id and is **not** workspace-scoped. The handler is
//! the isolation boundary — a resource whose `workspace_id` differs from
//! the caller's authorized workspace is reported as `404 Not Found`,
//! indistinguishable from a missing row (no cross-tenant existence or
//! content leak). A soft-deleted row and an unparsable id are likewise
//! 404.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use nebula_core::{ResourceId, TenantContext};
use nebula_storage::repos::ResourceEntry;

use crate::{
    errors::{ApiError, ApiResult, ProblemDetails},
    models::{ListResourcesResponse, ResourceSummary},
    state::AppState,
};

/// Project a persisted [`ResourceEntry`] onto the non-secret
/// [`ResourceSummary`] DTO.
///
/// Centralised so every resource read path produces an identical
/// projection: the raw `config` blob is never surfaced (ADR-0028 §7) and
/// the id is the prefixed `res_<ULID>` encoding. The `bytes_to_id` step is
/// fallible (a stored id that is not exactly 16 bytes is a storage
/// invariant violation), so the result is a `Result`.
fn entry_to_summary(entry: ResourceEntry) -> Result<ResourceSummary, ApiError> {
    let id = nebula_storage::mapping::bytes_to_id(&entry.id)
        .map(|bytes| ResourceId::from_bytes(bytes).to_string())
        .map_err(ApiError::from)?;
    Ok(ResourceSummary {
        id,
        name: entry.display_name,
        kind: entry.kind,
        version: entry.version.to_string(),
        // Workflow attachment is not tracked by the resource store yet;
        // advertised honestly as empty rather than fabricated.
        attached_to_workflows: Vec::new(),
    })
}

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
        .map(entry_to_summary)
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(ListResourcesResponse { resources }))
}

/// `GET /api/v1/orgs/{org}/workspaces/{ws}/resources/{res}` — fetch one resource.
///
/// Returns a single resource definition scoped to the caller's workspace.
/// The raw `config` blob is never surfaced — only the non-secret summary
/// fields (ADR-0028 §7).
///
/// All of the following collapse to **404 Not Found**, deliberately
/// indistinguishable so no information leaks across tenants:
/// - unknown id;
/// - an unparsable id string (an id that is not a `res_<ULID>` cannot
///   name an existing resource — "not found", not a 400/500);
/// - a resource owned by a *different* workspace (`ResourceRepo::get` is
///   keyed purely by id and is not workspace-scoped, so the handler
///   enforces tenant isolation here — neither the content nor the
///   existence of another tenant's resource is revealed);
/// - a soft-deleted row (a tombstone is not a resource).
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{ws}/resources/{res}",
    tag = "workspaces.resources",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("res" = String, Path, description = "Resource identifier (`res_<ULID>`)."),
    ),
    responses(
        (status = 200, description = "Resource definition (no raw config).", body = ResourceSummary),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Resource does not exist (also returned for a resource in another workspace, a soft-deleted resource, or an unparsable id — no cross-tenant leak).", body = ProblemDetails),
        (status = 500, description = "Resource repository error.", body = ProblemDetails),
        (status = 503, description = "Resource catalog backend is not configured on this instance.", body = ProblemDetails),
    ),
)]
pub async fn get_resource(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws, res)): Path<(String, String, String)>,
) -> ApiResult<Json<ResourceSummary>> {
    let workspace_id = tenant
        .require_workspace()
        .map_err(|_| ApiError::Forbidden("workspace context required".to_string()))?;
    let ws_bytes = workspace_id.as_bytes();

    let repo = state.resource_repo.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("Resource catalog backend not configured".into())
    })?;

    // An unparsable id cannot name an existing resource. Surfacing it as
    // 404 (not 400) keeps malformed-id and cross-tenant lookups
    // indistinguishable, so probing ids never leaks which ones are
    // structurally valid.
    let resource_id = ResourceId::parse(&res)
        .map_err(|_| ApiError::NotFound(format!("Resource {res} not found")))?;

    let entry = repo
        .get(resource_id.as_bytes().as_slice())
        .await?
        .filter(|entry| {
            // Tenant isolation + soft-delete tombstone collapse to the
            // same "not found" outcome as a genuinely absent row: a
            // resource in another workspace, or a deleted one, must be
            // indistinguishable from non-existence.
            entry.workspace_id.as_slice() == ws_bytes && entry.deleted_at.is_none()
        })
        .ok_or_else(|| ApiError::NotFound(format!("Resource {res} not found")))?;

    Ok(Json(entry_to_summary(entry)?))
}
