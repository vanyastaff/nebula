//! Runtime access guard for tenant-scoped routes.

use axum::{extract::Request, middleware::Next, response::Response};
use nebula_core::{Permission, TenantContext};

use crate::{error::ApiError, middleware::auth::AuthContext};

/// Require `permission` before allowing a protected route to run.
pub async fn require_permission(
    permission: Permission,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    {
        let auth = request
            .extensions()
            .get::<AuthContext>()
            .ok_or_else(|| ApiError::Unauthorized("not authenticated".to_string()))?;
        let tenant = request
            .extensions()
            .get::<TenantContext>()
            .ok_or_else(|| ApiError::Internal("tenant context not available".to_string()))?;

        tenant.require(permission)?;
        auth.grant
            .require(permission)
            .map_err(|err| ApiError::Forbidden(err.to_string()))?;
    }

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use nebula_core::{
        OrgId, OrgRole, Permission, Principal, TenantContext, WorkspaceId, WorkspaceRole,
    };
    use tower::ServiceExt;

    use super::require_permission;
    use crate::{
        access::Grant,
        middleware::auth::{AuthContext, AuthMethod},
    };

    fn protected_router(permission: Permission) -> Router {
        Router::new()
            .route("/", get(|| async { StatusCode::NO_CONTENT }))
            .layer(middleware::from_fn(move |request, next| {
                require_permission(permission, request, next)
            }))
    }

    fn request(auth: AuthContext, tenant: TenantContext) -> Request<Body> {
        let mut request = Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("test request must be valid");
        request.extensions_mut().insert(auth);
        request.extensions_mut().insert(tenant);
        request
    }

    fn auth_with_grant(grant: Grant) -> AuthContext {
        AuthContext {
            principal: Principal::System,
            auth_method: AuthMethod::ApiKey,
            grant,
        }
    }

    fn tenant_with_workspace_role(workspace_role: Option<WorkspaceRole>) -> TenantContext {
        TenantContext {
            org_id: OrgId::new(),
            workspace_id: Some(WorkspaceId::new()),
            principal: Principal::System,
            org_role: Some(OrgRole::OrgMember),
            workspace_role,
        }
    }

    #[tokio::test]
    async fn guard_allows_request_when_tenant_role_and_grant_allow_permission() {
        let response = protected_router(Permission::WorkflowRead)
            .oneshot(request(
                auth_with_grant(Grant::PatScoped(BTreeSet::from([Permission::WorkflowRead]))),
                tenant_with_workspace_role(Some(WorkspaceRole::WorkspaceViewer)),
            ))
            .await
            .expect("router must respond");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn guard_denies_request_when_grant_lacks_permission() {
        let response = protected_router(Permission::WorkflowRead)
            .oneshot(request(
                auth_with_grant(Grant::PatScoped(BTreeSet::new())),
                tenant_with_workspace_role(Some(WorkspaceRole::WorkspaceViewer)),
            ))
            .await
            .expect("router must respond");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn guard_denies_request_when_tenant_role_lacks_permission() {
        let response = protected_router(Permission::WorkflowRead)
            .oneshot(request(
                auth_with_grant(Grant::PatScoped(BTreeSet::from([Permission::WorkflowRead]))),
                tenant_with_workspace_role(None),
            ))
            .await
            .expect("router must respond");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
