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
use nebula_core::{ResolvedIds, TenantContext, role::effective_workspace_role};

use crate::{errors::ApiError, middleware::auth::AuthContext, state::AppState};

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

    // Load org role
    let org_role = match &state.membership_store {
        Some(store) => store.get_org_role(org_id, &auth_ctx.principal).await?,
        None => {
            // No membership store configured — in dev/test mode, allow through.
            None
        },
    };

    // If user has no org role at all, return 404 (enumeration prevention).
    // Exception: skip this check when no membership store is configured (dev mode).
    if state.membership_store.is_some() && org_role.is_none() {
        return Err(ApiError::NotFound("not found".to_string()));
    }

    // Load workspace role if workspace is resolved
    let workspace_role = if let Some(ws_id) = resolved.workspace_id {
        let explicit_role = match &state.membership_store {
            Some(store) => store.get_workspace_role(ws_id, &auth_ctx.principal).await?,
            None => None,
        };
        let effective = effective_workspace_role(org_role, explicit_role);

        // If user has org access but no effective workspace role, return 404
        if state.membership_store.is_some() && effective.is_none() {
            return Err(ApiError::NotFound("not found".to_string()));
        }
        effective
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
