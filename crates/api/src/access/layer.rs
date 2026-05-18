//! Runtime access guard for tenant-scoped routes.

use axum::{extract::Request, middleware::Next, response::Response};
use nebula_core::{Permission, TenantContext};
use tracing::Instrument;

use crate::{error::ApiError, middleware::auth::AuthContext};

/// Require `permission` before allowing a protected route to run.
pub async fn require_permission(
    permission: Permission,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let span = tracing::info_span!(
        "access.require_permission",
        permission = ?permission,
        auth.method = tracing::field::Empty,
        tenant.org_id = tracing::field::Empty,
        tenant.workspace_id = tracing::field::Empty,
        outcome = tracing::field::Empty,
    );

    async move {
        {
            let current_span = tracing::Span::current();
            let Some(auth) = request.extensions().get::<AuthContext>() else {
                current_span.record("outcome", "missing_auth_context");
                tracing::warn!("access denied: missing auth context");
                return Err(ApiError::Unauthorized("not authenticated".to_string()));
            };
            current_span.record("auth.method", tracing::field::debug(auth.auth_method));

            let Some(tenant) = request.extensions().get::<TenantContext>() else {
                current_span.record("outcome", "missing_tenant_context");
                tracing::error!("access invariant failed: tenant context not available");
                return Err(ApiError::Internal(
                    "tenant context not available".to_string(),
                ));
            };
            current_span.record("tenant.org_id", tracing::field::debug(&tenant.org_id));
            current_span.record(
                "tenant.workspace_id",
                tracing::field::debug(&tenant.workspace_id),
            );

            if let Err(err) = tenant.require(permission) {
                current_span.record("outcome", "tenant_denied");
                tracing::warn!(error = %err, "access denied by tenant role");
                return Err(err.into());
            }

            if let Err(err) = auth.grant.require(permission) {
                current_span.record("outcome", "grant_denied");
                tracing::warn!(error = %err, "access denied by auth grant");
                return Err(ApiError::Forbidden(err.to_string()));
            }

            current_span.record("outcome", "granted");
            tracing::debug!("access granted");
        }

        Ok(next.run(request).await)
    }
    .instrument(span)
    .await
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

    fn bare_request() -> Request<Body> {
        Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("test request must be valid")
    }

    fn request(auth: AuthContext, tenant: TenantContext) -> Request<Body> {
        let mut request = bare_request();
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

    #[tokio::test]
    async fn guard_denies_request_when_missing_auth_context() {
        let mut request = bare_request();
        request
            .extensions_mut()
            .insert(tenant_with_workspace_role(Some(
                WorkspaceRole::WorkspaceViewer,
            )));

        let response = protected_router(Permission::WorkflowRead)
            .oneshot(request)
            .await
            .expect("router must respond");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn guard_errors_when_missing_tenant_context() {
        let mut request = bare_request();
        request
            .extensions_mut()
            .insert(auth_with_grant(Grant::PatScoped(BTreeSet::from([
                Permission::WorkflowRead,
            ]))));

        let response = protected_router(Permission::WorkflowRead)
            .oneshot(request)
            .await
            .expect("router must respond");

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
