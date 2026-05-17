//! Workspace-scoped route assembly — authenticated + tenant-scoped.
//!
//! Tenant-prefix nesting concern: this module merges the workflow /
//! execution / resource / credential domain route tables into the
//! `/api/v1/orgs/{org}/workspaces/{ws}/*` group behind auth + tenancy +
//! RBAC middleware (applied in [`crate::domain::create_routes`]).
//!
//! `execution::terminate_execution` is fully implemented end-to-end via
//! the durable control queue (canon §12.2; `ControlCommand::Terminate` →
//! `EngineControlDispatch::dispatch_terminate`, ADR-0008 A3 / ADR-0016) —
//! it is no longer a stub. The resource catalog surface
//! (`resource::{list,get,create,update,delete}_resource` +
//! `resource::get_resource_status`) is likewise fully implemented:
//! config-CRUD + CAS + a read-only runtime-status projection (ADR-0067) —
//! it is no longer a stub. `execution::restart_execution` is still
//! stubbed (501) and carries `#[deprecated]` so the OpenAPI spec flags it
//! per ADR-0047 Stub Endpoint Policy. The deprecation lint is silenced at
//! module level — that handler is intentionally mounted so the route
//! table stays in sync with the published spec.
#![allow(deprecated)]

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    domain::{
        credential::handler as credential, execution::handler as execution,
        resource::handler as resource, workflow::handler as workflow,
    },
    state::AppState,
};

/// Workspace-scoped routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Workflows
        .routes(routes!(
            workflow::list_workflows,
            workflow::create_workflow
        ))
        .routes(routes!(
            workflow::get_workflow,
            workflow::update_workflow,
            workflow::delete_workflow
        ))
        .routes(routes!(workflow::activate_workflow))
        .routes(routes!(workflow::execute_workflow))
        .routes(routes!(workflow::validate_workflow_handler))
        // Executions
        .routes(routes!(
            execution::list_executions_for_workflow,
            execution::start_execution
        ))
        .routes(routes!(execution::list_executions))
        .routes(routes!(
            execution::get_execution,
            execution::cancel_execution
        ))
        .routes(routes!(execution::terminate_execution))
        .routes(routes!(execution::restart_execution))
        // Resources. Config-CRUD collection + by-id, then the READ-ONLY
        // runtime-status projection. Resource lifecycle
        // (acquire/release/drain/reload) is engine-owned and deliberately
        // NOT exposed over HTTP (INTEGRATION_MODEL §13.1): `{res}/status`
        // is the ONLY `{res}/...` sub-route, and it is a GET only — there
        // is intentionally no acquire/release/drain route.
        .routes(routes!(resource::list_resources, resource::create_resource))
        .routes(routes!(
            resource::get_resource,
            resource::update_resource,
            resource::delete_resource
        ))
        .routes(routes!(resource::get_resource_status))
        // Credentials (Plane B — ADR-0031). Literal paths first, then
        // collection, then parameterized `{cred}`, then sub-resources.
        .routes(routes!(credential::resolve_credential))
        .routes(routes!(credential::continue_resolve_credential))
        .routes(routes!(
            credential::list_credentials,
            credential::create_credential
        ))
        .routes(routes!(
            credential::get_credential,
            credential::update_credential,
            credential::delete_credential
        ))
        .routes(routes!(credential::test_credential))
        .routes(routes!(credential::refresh_credential))
        .routes(routes!(credential::revoke_credential))
}
