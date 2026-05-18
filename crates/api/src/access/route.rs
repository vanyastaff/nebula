//! Route and OpenAPI helpers for Access Kernel protected routes.

use std::fmt;

use nebula_core::Permission;
use serde_json::{Value, json};
use utoipa::openapi::{
    OpenApi,
    extensions::Extensions,
    path::{Operation, PathItem},
};
use utoipa_axum::router::UtoipaMethodRouter;

use crate::access::{UNSUPPORTED_PERMISSION_SCOPE, layer::require_permission, permission_scope};

/// OpenAPI operation extension containing the required Access Kernel scope.
pub const REQUIRED_PERMISSION_EXTENSION: &str = "x-required-permission";

/// Attach runtime and OpenAPI access metadata to a method router.
pub fn protected<S>(permission: Permission, routes: UtoipaMethodRouter<S>) -> UtoipaMethodRouter<S>
where
    S: Send + Sync + Clone + 'static,
{
    let scope = permission_scope(permission);
    assert_ne!(
        scope, UNSUPPORTED_PERMISSION_SCOPE,
        "unsupported permission {permission:?} cannot protect a public route"
    );

    let (schemas, mut paths, method_router) = routes;
    annotate_paths(&mut paths, scope);
    let method_router = method_router.layer(axum::middleware::from_fn(move |request, next| {
        require_permission(permission, request, next)
    }));

    (schemas, paths, method_router)
}

/// Access coverage failures found in tenant-scoped OpenAPI operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessCoverageError {
    failures: Vec<String>,
}

impl fmt::Display for AccessCoverageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.failures.join("; "))
    }
}

impl std::error::Error for AccessCoverageError {}

/// Assert that every tenant-scoped OpenAPI operation declares access metadata.
pub fn assert_tenant_access_coverage(openapi: &OpenApi) -> Result<(), AccessCoverageError> {
    let mut failures = Vec::new();

    for (path, item) in &openapi.paths.paths {
        if !is_tenant_scoped_path(path) {
            continue;
        }

        for (method, operation) in operations(item) {
            match required_permission_extension(operation) {
                Some(Value::String(scope)) if scope != UNSUPPORTED_PERMISSION_SCOPE => {},
                Some(Value::String(scope)) => failures.push(format!(
                    "{path} {method} has unsupported {REQUIRED_PERMISSION_EXTENSION} value {scope}"
                )),
                Some(_) => failures.push(format!(
                    "{path} {method} {REQUIRED_PERMISSION_EXTENSION} must be a string"
                )),
                None => failures.push(format!(
                    "{path} {method} is missing string {REQUIRED_PERMISSION_EXTENSION}"
                )),
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(AccessCoverageError { failures })
    }
}

fn annotate_paths(paths: &mut utoipa::openapi::path::Paths, scope: &str) {
    for item in paths.paths.values_mut() {
        for operation in operations_mut(item) {
            operation
                .extensions
                .get_or_insert_with(Extensions::default)
                .insert(REQUIRED_PERMISSION_EXTENSION.to_string(), json!(scope));
        }
    }
}

fn is_tenant_scoped_path(path: &str) -> bool {
    path.starts_with("/api/v1/orgs/{org}")
}

fn required_permission_extension(operation: &Operation) -> Option<&Value> {
    operation
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(REQUIRED_PERMISSION_EXTENSION))
}

fn operations(item: &PathItem) -> impl Iterator<Item = (&'static str, &Operation)> {
    [
        ("GET", item.get.as_ref()),
        ("PUT", item.put.as_ref()),
        ("POST", item.post.as_ref()),
        ("DELETE", item.delete.as_ref()),
        ("OPTIONS", item.options.as_ref()),
        ("HEAD", item.head.as_ref()),
        ("PATCH", item.patch.as_ref()),
        ("TRACE", item.trace.as_ref()),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.map(|operation| (method, operation)))
}

fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = &mut Operation> {
    [
        item.get.as_mut(),
        item.put.as_mut(),
        item.post.as_mut(),
        item.delete.as_mut(),
        item.options.as_mut(),
        item.head.as_mut(),
        item.patch.as_mut(),
        item.trace.as_mut(),
    ]
    .into_iter()
    .flatten()
}

#[cfg(test)]
mod tests {
    use axum::routing::MethodRouter;
    use nebula_core::Permission;
    use serde_json::{Value, json};
    use utoipa::openapi::{
        OpenApiBuilder, RefOr,
        extensions::Extensions,
        path::{HttpMethod, Operation, OperationBuilder, PathItem, PathsBuilder},
        schema::{Ref, Schema},
    };
    use utoipa_axum::{
        router::{OpenApiRouter, UtoipaMethodRouter},
        routes,
    };

    use super::{REQUIRED_PERMISSION_EXTENSION, assert_tenant_access_coverage, protected};
    use crate::access::{UNSUPPORTED_PERMISSION_SCOPE, permission_scope};

    fn openapi_with_path(path: &str, operation: Operation) -> utoipa::openapi::OpenApi {
        OpenApiBuilder::new()
            .paths(
                PathsBuilder::new()
                    .path(path, PathItem::new(HttpMethod::Get, operation))
                    .build(),
            )
            .build()
    }

    fn operation_with_extension(value: Value) -> Operation {
        OperationBuilder::new()
            .extensions(Some(Extensions::from_iter([(
                REQUIRED_PERMISSION_EXTENSION.to_string(),
                value,
            )])))
            .build()
    }

    #[utoipa::path(
        get,
        path = "/orgs/{org}/things",
        responses((status = 200, description = "ok"))
    )]
    async fn dummy_tenant_route() {}

    #[test]
    fn tenant_path_without_required_permission_extension_fails_coverage() {
        let openapi = openapi_with_path(
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows",
            Operation::new(),
        );

        let err = assert_tenant_access_coverage(&openapi)
            .expect_err("tenant operation without extension must fail");

        let message = err.to_string();
        assert!(message.contains("/api/v1/orgs/{org}/workspaces/{ws}/workflows GET"));
        assert!(message.contains(REQUIRED_PERMISSION_EXTENSION));
    }

    #[test]
    fn tenant_path_with_required_permission_extension_passes_coverage() {
        let openapi = openapi_with_path(
            "/api/v1/orgs/{org}/workspaces/{ws}/workflows",
            operation_with_extension(json!("workflows:read")),
        );

        assert_eq!(assert_tenant_access_coverage(&openapi), Ok(()));
    }

    #[test]
    fn non_tenant_path_is_ignored_by_coverage() {
        let openapi = openapi_with_path("/api/v1/me", Operation::new());

        assert_eq!(assert_tenant_access_coverage(&openapi), Ok(()));
    }

    #[test]
    fn unsupported_required_permission_extension_fails_coverage() {
        let openapi = openapi_with_path(
            "/api/v1/orgs/{org}",
            operation_with_extension(json!(UNSUPPORTED_PERMISSION_SCOPE)),
        );

        let err = assert_tenant_access_coverage(&openapi)
            .expect_err("unsupported access extension must fail");

        let message = err.to_string();
        assert!(message.contains("/api/v1/orgs/{org} GET"));
        assert!(message.contains(UNSUPPORTED_PERMISSION_SCOPE));
    }

    #[test]
    fn non_string_required_permission_extension_fails_coverage() {
        let openapi = openapi_with_path(
            "/api/v1/orgs/{org}",
            operation_with_extension(json!(["orgs:read"])),
        );

        let err = assert_tenant_access_coverage(&openapi)
            .expect_err("non-string access extension must fail");

        assert!(err.to_string().contains("must be a string"));
    }

    #[test]
    fn protected_annotates_every_operation_with_required_permission() {
        let paths = PathsBuilder::new()
            .path(
                "/workflows",
                PathItem::new(HttpMethod::Get, Operation::new()),
            )
            .path(
                "/workflows/{workflow}",
                PathItem::from_http_methods(
                    [HttpMethod::Get, HttpMethod::Delete],
                    Operation::new(),
                ),
            )
            .build();
        let permission = Permission::WorkflowRead;

        let (_, annotated_paths, _) =
            protected(permission, (Vec::new(), paths, MethodRouter::<()>::new()));

        for item in annotated_paths.paths.values() {
            for operation in [item.get.as_ref(), item.delete.as_ref()]
                .into_iter()
                .flatten()
            {
                let extension = operation
                    .extensions
                    .as_ref()
                    .and_then(|extensions| extensions.get(REQUIRED_PERMISSION_EXTENSION));
                assert_eq!(extension, Some(&json!(permission_scope(permission))));
            }
        }
    }

    #[test]
    fn protected_preserves_schemas_while_annotating_paths() {
        let schemas = vec![(
            "ProtectedSchema".to_string(),
            RefOr::Ref(Ref::from_schema_name("ProtectedSchema")),
        )];
        let paths = PathsBuilder::new()
            .path(
                "/workflows",
                PathItem::new(HttpMethod::Get, Operation::new()),
            )
            .build();
        let routes: UtoipaMethodRouter<()> = (schemas.clone(), paths, MethodRouter::<()>::new());

        let (protected_schemas, protected_paths, _) = protected(Permission::WorkflowRead, routes);

        assert!(protected_schemas == schemas);
        assert_eq!(
            protected_paths
                .get_path_operation("/workflows", HttpMethod::Get)
                .and_then(|operation| operation.extensions.as_ref())
                .and_then(|extensions| extensions.get(REQUIRED_PERMISSION_EXTENSION)),
            Some(&json!("workflows:read"))
        );
    }

    #[test]
    #[should_panic(expected = "unsupported permission")]
    fn protected_panics_for_permission_without_pat_scope_mapping() {
        assert_eq!(
            permission_scope(Permission::WorkspaceMemberRead),
            UNSUPPORTED_PERMISSION_SCOPE
        );

        let routes: UtoipaMethodRouter<()> = (
            Vec::<(String, RefOr<Schema>)>::new(),
            PathsBuilder::new()
                .path("/members", PathItem::new(HttpMethod::Get, Operation::new()))
                .build(),
            MethodRouter::<()>::new(),
        );

        let _ = protected(Permission::WorkspaceMemberRead, routes);
    }

    #[test]
    fn coverage_passes_after_openapi_router_nesting_adds_api_v1_prefix() {
        let openapi = OpenApiRouter::<()>::new()
            .nest(
                "/api/v1",
                OpenApiRouter::new()
                    .routes(protected(Permission::OrgRead, routes!(dummy_tenant_route))),
            )
            .into_openapi();

        assert!(
            openapi
                .paths
                .get_path_operation("/api/v1/orgs/{org}/things", HttpMethod::Get)
                .is_some()
        );
        assert_eq!(assert_tenant_access_coverage(&openapi), Ok(()));
    }
}
