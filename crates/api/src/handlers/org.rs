//! Organization-level endpoint handlers.
//! These require auth + org-level tenancy (ResolvedIds with org_id).

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use nebula_core::TenantContext;

use crate::{
    errors::{ApiError, ApiResult},
    state::AppState,
};

/// GET /api/v1/orgs/{org}
pub async fn get_org(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Return organization details
    Err(ApiError::Internal("not implemented".to_string()))
}

/// PATCH /api/v1/orgs/{org}
pub async fn update_org(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Update org settings (requires OrgAdmin+)
    tenant.require(nebula_core::Permission::OrgUpdate)?;
    Err(ApiError::Internal("not implemented".to_string()))
}

/// DELETE /api/v1/orgs/{org}
pub async fn delete_org(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Delete organization (requires OrgOwner)
    tenant.require(nebula_core::Permission::OrgDelete)?;
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/orgs/{org}/members
pub async fn list_members(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: List org members with roles
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/orgs/{org}/members
pub async fn invite_member(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Invite user to org
    tenant.require(nebula_core::Permission::MemberInvite)?;
    Err(ApiError::Internal("not implemented".to_string()))
}

/// DELETE /api/v1/orgs/{org}/members/{principal}
pub async fn remove_member(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _principal_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Remove member from org
    tenant.require(nebula_core::Permission::MemberRemove)?;
    Err(ApiError::Internal("not implemented".to_string()))
}

/// GET /api/v1/orgs/{org}/service-accounts
pub async fn list_service_accounts(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: List service accounts for org
    Err(ApiError::Internal("not implemented".to_string()))
}

/// POST /api/v1/orgs/{org}/service-accounts
pub async fn create_service_account(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Create service account
    tenant.require(nebula_core::Permission::ServiceAccountManage)?;
    Err(ApiError::Internal("not implemented".to_string()))
}

/// DELETE /api/v1/orgs/{org}/service-accounts/{sa}
pub async fn delete_service_account(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _sa_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    // TODO: Delete service account
    tenant.require(nebula_core::Permission::ServiceAccountManage)?;
    Err(ApiError::Internal("not implemented".to_string()))
}
