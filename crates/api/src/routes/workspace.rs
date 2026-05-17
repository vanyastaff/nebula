//! Workspace-scoped routes — authenticated + tenant-scoped.
//!
//! All routes under `/api/v1/orgs/{org}/workspaces/{ws}/*` are behind
//! auth + tenancy + RBAC middleware layers.
//!
//! `execution::terminate_execution` and `execution::restart_execution`
//! are still stubbed (501) and carry `#[deprecated]` so the OpenAPI spec
//! flags them per ADR-0047 Stub Endpoint Policy. The deprecation lint is
//! silenced at module level — these handlers are intentionally mounted so
//! the route table stays in sync with the published spec.
#![allow(deprecated)]

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// Workspace-scoped routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Workflows
        .routes(routes!(
            handlers::workflow::list_workflows,
            handlers::workflow::create_workflow
        ))
        .routes(routes!(
            handlers::workflow::get_workflow,
            handlers::workflow::update_workflow,
            handlers::workflow::delete_workflow
        ))
        .routes(routes!(handlers::workflow::activate_workflow))
        .routes(routes!(handlers::workflow::execute_workflow))
        // Executions
        .routes(routes!(
            handlers::execution::list_executions_for_workflow,
            handlers::execution::start_execution
        ))
        .routes(routes!(handlers::execution::list_executions))
        .routes(routes!(
            handlers::execution::get_execution,
            handlers::execution::cancel_execution
        ))
        .routes(routes!(handlers::execution::terminate_execution))
        .routes(routes!(handlers::execution::restart_execution))
        // Resources
        .routes(routes!(handlers::resource::list_resources))
        // Credentials (Plane B — ADR-0031). Literal paths first, then
        // collection, then parameterized `{cred}`, then sub-resources.
        .routes(routes!(handlers::credential::resolve_credential))
        .routes(routes!(handlers::credential::continue_resolve_credential))
        .routes(routes!(
            handlers::credential::list_credentials,
            handlers::credential::create_credential
        ))
        .routes(routes!(
            handlers::credential::get_credential,
            handlers::credential::update_credential,
            handlers::credential::delete_credential
        ))
        .routes(routes!(handlers::credential::test_credential))
        .routes(routes!(handlers::credential::refresh_credential))
        .routes(routes!(handlers::credential::revoke_credential))
}
