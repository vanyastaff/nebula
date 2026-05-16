//! Workspace-scoped route assembly — authenticated + tenant-scoped.
//!
//! Tenant-prefix nesting concern: this module merges the workflow /
//! execution / resource / credential domain route tables into the
//! `/api/v1/orgs/{org}/workspaces/{ws}/*` group behind auth + tenancy +
//! RBAC middleware (applied in [`crate::domain::create_routes`]).
//!
//! `resource::list_resources`, `execution::terminate_execution`, and
//! `execution::restart_execution` are still stubbed (501) and carry
//! `#[deprecated]` so the OpenAPI spec flags them per ADR-0047 Stub
//! Endpoint Policy. The deprecation lint is silenced at module level —
//! these handlers are intentionally mounted so the route table stays in
//! sync with the published spec.
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
        // Resources
        .routes(routes!(resource::list_resources))
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
