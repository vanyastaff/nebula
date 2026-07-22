//! RBAC middleware.
//!
//! Loads org and workspace membership roles for the authenticated principal,
//! computes effective roles, and constructs [`TenantContext`].
//!
//! Policy: 404 when user has no access to tenant (prevents enumeration),
//! 403 when user has access but insufficient role.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use nebula_core::{
    OrgRole, ResolvedIds, TenantContext, WorkspaceGrant, WorkspaceRole,
    role::effective_workspace_role,
};

use crate::{
    error::ApiError,
    middleware::auth::AuthContext,
    state::{AppState, TenantMembershipSnapshot},
};

/// RBAC middleware — must run AFTER auth and tenancy middleware.
///
/// Reads [`AuthContext`] and [`ResolvedIds`] from request extensions,
/// loads membership roles via [`MembershipStore`], and inserts a fully
/// resolved [`TenantContext`] into extensions.
///
/// [`MembershipStore`]: crate::state::MembershipStore
pub async fn rbac_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // Extract auth context — if missing, the auth middleware already rejected
    let auth_ctx = request
        .extensions()
        .get::<AuthContext>()
        .cloned()
        .ok_or(ApiError::Unauthorized("not authenticated".to_string()))?;

    // Extract resolved IDs — if missing, this is a non-tenant route (shouldn't have RBAC)
    let resolved = request
        .extensions()
        .get::<ResolvedIds>()
        .cloned()
        .ok_or(ApiError::Internal("resolved IDs not available".to_string()))?;

    let org_id = resolved
        .org_id
        .ok_or(ApiError::Internal("org_id not resolved".to_string()))?;

    let insecure_bypass_without_store =
        state.allow_insecure_tenant_rbac_bypass() && state.membership_store.is_none();

    // Load one consistent org/workspace role snapshot. Implementations must
    // not splice two independently observed membership states together.
    let membership = match &state.membership_store {
        Some(store) => {
            store
                .get_tenant_membership(org_id, resolved.workspace_id, &auth_ctx.principal)
                .await?
        },
        None if insecure_bypass_without_store => TenantMembershipSnapshot {
            org_role: Some(OrgRole::OrgOwner),
            workspace_role: resolved.workspace_id.map(|_| WorkspaceRole::WorkspaceAdmin),
        },
        None => {
            return Err(ApiError::ServiceUnavailable(
                "membership store not configured; tenant routes are disabled".to_string(),
            ));
        },
    };
    let org_role = membership.org_role;

    // If user has no org role at all, return 404 (enumeration prevention).
    if org_role.is_none() {
        return Err(ApiError::NotFound("not found".to_string()));
    }

    // Load workspace role if workspace is resolved
    let workspace_role = if let Some(ws_id) = resolved.workspace_id {
        let effective = effective_workspace_role(org_role, membership.workspace_role);

        // If user has org access but no effective workspace role, return 404
        if effective.is_none() {
            return Err(ApiError::NotFound("not found".to_string()));
        }
        effective.map(|role| WorkspaceGrant::new(ws_id, role))
    } else {
        None
    };

    // Build TenantContext
    let tenant_ctx = TenantContext {
        org_id,
        workspace_id: resolved.workspace_id,
        principal: auth_ctx.principal,
        org_role,
        workspace_role,
    };

    request.extensions_mut().insert(tenant_ctx);

    Ok(next.run(request).await)
}
