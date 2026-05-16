//! Organization-level endpoint handlers.
//! These require auth + org-level tenancy (ResolvedIds with org_id).
//!
//! Every handler in this module is currently a 501-equivalent stub (audit
//! class (c)); the OpenAPI annotations describe the **planned** body shape
//! per ADR-0047 Stub Endpoint Policy. Tag suffix `(planned)` flags the
//! group in Swagger UI; once the underlying RBAC store milestone closes
//! the only diff is removing `deprecated = true` and the 501 response.
//!
//! Note: `tenant.require(...)` RBAC gates ARE real today — the spec
//! declares 403 alongside the 501 outcome to reflect the actual runtime
//! contract.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use nebula_core::TenantContext;

use crate::{
    domain::{
        org::dto::{
            CreateServiceAccountRequest, CreateServiceAccountResponse, InviteMemberRequest,
            InviteMemberResponse, MembersResponse, OrgResponse, ServiceAccountsResponse,
            UpdateOrgRequest,
        },
        shared::AckResponse,
    },
    error::{ApiError, ApiResult, ProblemDetails},
    state::AppState,
};

/// `GET /api/v1/orgs/{org}` — organisation details.
#[utoipa::path(
    get,
    path = "/orgs/{org}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = OrgResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 404, description = "Organisation not found.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn get_org(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `PATCH /api/v1/orgs/{org}` — update organisation settings (requires `OrgUpdate`).
#[utoipa::path(
    patch,
    path = "/orgs/{org}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    request_body = UpdateOrgRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = OrgResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `OrgUpdate` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn update_org(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::OrgUpdate)?;
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `DELETE /api/v1/orgs/{org}` — delete organisation (requires `OrgDelete`).
#[utoipa::path(
    delete,
    path = "/orgs/{org}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `OrgDelete` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn delete_org(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::OrgDelete)?;
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `GET /api/v1/orgs/{org}/members` — list organisation members.
#[utoipa::path(
    get,
    path = "/orgs/{org}/members",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = MembersResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller is not a member of this organisation.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn list_members(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `POST /api/v1/orgs/{org}/members` — invite a new member (requires `MemberInvite`).
#[utoipa::path(
    post,
    path = "/orgs/{org}/members",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    request_body = InviteMemberRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = InviteMemberResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `MemberInvite` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn invite_member(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::MemberInvite)?;
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `DELETE /api/v1/orgs/{org}/members/{principal}` — remove a member
/// (requires `MemberRemove`).
#[utoipa::path(
    delete,
    path = "/orgs/{org}/members/{principal}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("principal" = String, Path, description = "Principal identifier (`user_<ULID>`)."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `MemberRemove` permission (gate is enforced today).", body = ProblemDetails),
        (status = 404, description = "Member does not exist in this organisation.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn remove_member(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _principal_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::MemberRemove)?;
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `GET /api/v1/orgs/{org}/service-accounts` — list service accounts.
#[utoipa::path(
    get,
    path = "/orgs/{org}/service-accounts",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = ServiceAccountsResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller is not a member of this organisation.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn list_service_accounts(
    State(_state): State<AppState>,
    Extension(_tenant): Extension<TenantContext>,
) -> ApiResult<Json<serde_json::Value>> {
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `POST /api/v1/orgs/{org}/service-accounts` — create a service account
/// (requires `ServiceAccountManage`).
#[utoipa::path(
    post,
    path = "/orgs/{org}/service-accounts",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
    ),
    request_body = CreateServiceAccountRequest,
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = CreateServiceAccountResponse),
        (status = 400, description = "Validation error.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `ServiceAccountManage` permission (gate is enforced today).", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn create_service_account(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Json(_body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::ServiceAccountManage)?;
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}

/// `DELETE /api/v1/orgs/{org}/service-accounts/{sa}` — delete a service
/// account (requires `ServiceAccountManage`).
#[utoipa::path(
    delete,
    path = "/orgs/{org}/service-accounts/{sa}",
    tag = "orgs (planned)",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("sa" = String, Path, description = "Service-account identifier (`sa_<ULID>`)."),
    ),
    responses(
        (status = 501, description = "Not yet implemented; tracked under RBAC store milestone.", body = AckResponse),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller lacks `ServiceAccountManage` permission (gate is enforced today).", body = ProblemDetails),
        (status = 404, description = "Service account does not exist.", body = ProblemDetails),
    ),
)]
#[deprecated(note = "Stub: returns 501 once RBAC store milestone closes.")]
pub async fn delete_service_account(
    State(_state): State<AppState>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _sa_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    tenant.require(nebula_core::Permission::ServiceAccountManage)?;
    Err(ApiError::NotImplemented(
        "handler stub — tracked under ADR-0047 Stub Endpoint Policy".to_string(),
    ))
}
