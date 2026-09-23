//! Workspace membership handlers.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use nebula_core::{TenantContext, WorkspaceId};

use super::dto::{UpsertWorkspaceMemberRequest, WorkspaceMemberSummary, WorkspaceMembersResponse};
use crate::{
    domain::{
        membership_support::{parse_principal, principal_id, store as membership_store},
        shared::{AckResponse, WorkspaceRoleDto},
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::AppState,
};

fn workspace_id(tenant: &TenantContext) -> Result<WorkspaceId, ApiError> {
    tenant.workspace_id.ok_or_else(|| {
        ApiError::Internal("workspace member handler reached without a workspace".to_owned())
    })
}

/// List explicit members of one parent-qualified workspace.
#[utoipa::path(
    get,
    path = "/orgs/{org}/workspaces/{workspace}/members",
    tag = "workspace members",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("workspace" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    responses(
        (status = 200, description = "Explicit workspace memberships.", body = WorkspaceMembersResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks workspace member read authority.", body = ProblemDetails),
        (status = 404, description = "Workspace not found or inaccessible.", body = ProblemDetails),
        (status = 503, description = "Membership authority unavailable.", body = ProblemDetails),
    ),
)]
pub async fn list_workspace_members(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<WorkspaceMembersResponse>> {
    tenant.require(nebula_core::Permission::WorkspaceMemberRead)?;
    let workspace_id = workspace_id(&tenant)?;
    let members = membership_store(&state)?
        .list_workspace_members(tenant.org_id, workspace_id)
        .await?;
    let members = members
        .into_iter()
        .map(|member| {
            Ok(WorkspaceMemberSummary {
                principal_id: principal_id(&member.principal)?,
                role: WorkspaceRoleDto::from(member.role),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let mut members = members;
    members.sort_unstable_by(|left, right| left.principal_id.cmp(&right.principal_id));

    tracing::info!(org_id = %tenant.org_id, workspace_id = %workspace_id, count = members.len(), "workspace members listed");
    Ok(Json(WorkspaceMembersResponse { members }))
}

/// Add or replace one explicit workspace grant.
#[utoipa::path(
    put,
    path = "/orgs/{org}/workspaces/{workspace}/members/{principal}",
    tag = "workspace members",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("workspace" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("principal" = String, Path, description = "User or service-account identity."),
    ),
    request_body = UpsertWorkspaceMemberRequest,
    responses(
        (status = 200, description = "Workspace membership added or replaced idempotently.", body = WorkspaceMemberSummary),
        (status = 400, description = "Invalid principal or role token.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks workspace member management authority.", body = ProblemDetails),
        (status = 404, description = "Workspace, organization member, or tenant access not found.", body = ProblemDetails),
        (status = 503, description = "Membership authority unavailable.", body = ProblemDetails),
    ),
)]
pub async fn upsert_workspace_member(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _workspace, raw_principal)): Path<(String, String, String)>,
    Json(body): Json<UpsertWorkspaceMemberRequest>,
) -> ApiResult<Json<WorkspaceMemberSummary>> {
    tenant.require(nebula_core::Permission::WorkspaceMemberManage)?;
    let workspace_id = workspace_id(&tenant)?;
    let principal = parse_principal(&raw_principal)?;
    let role = WorkspaceRoleDto::parse(&body.role.0).ok_or_else(|| {
        ApiError::validation_message("role must be one of viewer|runner|editor|admin".to_owned())
    })?;
    let store = membership_store(&state)?;

    // Workspace grants are subordinate to organization membership. The same
    // 404 is used for an absent target and an inaccessible tenant.
    if store
        .get_org_role(tenant.org_id, &principal)
        .await?
        .is_none()
    {
        return Err(ApiError::NotFound("member not found".to_owned()));
    }

    store
        .upsert_workspace_member(tenant.org_id, workspace_id, &principal, role)
        .await?;
    tracing::info!(org_id = %tenant.org_id, workspace_id = %workspace_id, principal = %raw_principal, role = WorkspaceRoleDto::token(role), "workspace member added or updated");
    Ok(Json(WorkspaceMemberSummary {
        principal_id: principal_id(&principal)?,
        role: WorkspaceRoleDto::from(role),
    }))
}

/// Remove one explicit workspace grant.
#[utoipa::path(
    delete,
    path = "/orgs/{org}/workspaces/{workspace}/members/{principal}",
    tag = "workspace members",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("workspace" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
        ("principal" = String, Path, description = "User or service-account identity."),
    ),
    responses(
        (status = 200, description = "Explicit workspace membership removed.", body = AckResponse),
        (status = 400, description = "Invalid principal id.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks workspace member management authority.", body = ProblemDetails),
        (status = 404, description = "Membership, workspace, or tenant access not found.", body = ProblemDetails),
        (status = 503, description = "Membership authority unavailable.", body = ProblemDetails),
    ),
)]
pub async fn remove_workspace_member(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _workspace, raw_principal)): Path<(String, String, String)>,
) -> ApiResult<Json<AckResponse>> {
    tenant.require(nebula_core::Permission::WorkspaceMemberManage)?;
    let workspace_id = workspace_id(&tenant)?;
    let principal = parse_principal(&raw_principal)?;
    let removed = membership_store(&state)?
        .remove_workspace_member(tenant.org_id, workspace_id, &principal)
        .await?;
    if !removed {
        return Err(ApiError::NotFound("member not found".to_owned()));
    }
    tracing::info!(org_id = %tenant.org_id, workspace_id = %workspace_id, principal = %raw_principal, "workspace member removed");
    Ok(Json(AckResponse::ok()))
}
