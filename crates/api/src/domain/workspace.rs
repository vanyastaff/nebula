//! Workspace-scoped route assembly — authenticated + tenant-scoped.
//!
//! Tenant-prefix nesting concern: this module merges the workflow /
//! execution / resource / credential domain route tables into the
//! `/api/v1/orgs/{org}/workspaces/{ws}/*` group behind auth + tenancy +
//! RBAC middleware (applied in [`crate::domain::create_routes`]).
//!
//! `execution::terminate_execution` is fully implemented end-to-end via
//! the durable control queue (durable control queue; `ControlCommand::Terminate` →
//! `EngineControlDispatch::dispatch_terminate`, control-queue terminate dispatch / cooperative cancel) —
//! it is no longer a stub. The resource catalog surface
//! (`resource::{list,get,create,update,delete}_resource` +
//! `resource::get_resource_status`) is likewise fully implemented:
//! config-CRUD + CAS + a read-only runtime-status projection (resource runtime status) —
//! it is no longer a stub. `execution::restart_execution` is still
//! stubbed (501) and carries `#[deprecated]` so the OpenAPI spec flags it
//! per stub endpoint policy. The deprecation lint is silenced at
//! module level — that handler is intentionally mounted so the route
//! table stays in sync with the published spec.
#![allow(deprecated)]

use nebula_core::Permission;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{
    access,
    domain::{
        credential::handler as credential, execution::handler as execution,
        resource::handler as resource, webhook::handler as webhook, workflow::handler as workflow,
    },
    state::AppState,
};

/// Workspace-scoped routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Workflows
        .routes(access::protected(
            Permission::WorkflowRead,
            routes!(workflow::list_workflows),
        ))
        .routes(access::protected(
            Permission::WorkflowRead,
            routes!(workflow::get_workflow),
        ))
        .routes(access::protected(
            Permission::WorkflowWrite,
            routes!(workflow::create_workflow),
        ))
        .routes(access::protected(
            Permission::WorkflowWrite,
            routes!(workflow::update_workflow),
        ))
        .routes(access::protected(
            Permission::WorkflowWrite,
            routes!(workflow::activate_workflow),
        ))
        .routes(access::protected(
            Permission::WorkflowWrite,
            routes!(workflow::validate_workflow_handler),
        ))
        .routes(access::protected(
            Permission::WorkflowDelete,
            routes!(workflow::delete_workflow),
        ))
        .routes(access::protected(
            Permission::WorkflowExecute,
            routes!(workflow::execute_workflow),
        ))
        // Executions
        .routes(access::protected(
            Permission::ExecutionRead,
            routes!(execution::list_executions_for_workflow),
        ))
        .routes(access::protected(
            Permission::ExecutionRead,
            routes!(execution::list_executions),
        ))
        .routes(access::protected(
            Permission::ExecutionRead,
            routes!(execution::get_execution),
        ))
        .routes(access::protected(
            Permission::WorkflowExecute,
            routes!(execution::start_execution),
        ))
        .routes(access::protected(
            Permission::ExecutionCancel,
            routes!(execution::cancel_execution),
        ))
        .routes(access::protected(
            Permission::ExecutionTerminate,
            routes!(execution::terminate_execution),
        ))
        .routes(access::protected(
            Permission::ExecutionRestart,
            routes!(execution::restart_execution),
        ))
        // Resources. Config-CRUD collection + by-id, then the READ-ONLY
        // runtime-status projection. Resource lifecycle
        // (acquire/release/drain/reload) is engine-owned and deliberately
        // NOT exposed over HTTP (INTEGRATION_MODEL integration seam.1): `{res}/status`
        // is the ONLY `{res}/...` sub-route, and it is a GET only — there
        // is intentionally no acquire/release/drain route.
        .routes(access::protected(
            Permission::ResourceRead,
            routes!(resource::list_resources),
        ))
        .routes(access::protected(
            Permission::ResourceRead,
            routes!(resource::get_resource),
        ))
        .routes(access::protected(
            Permission::ResourceRead,
            routes!(resource::get_resource_status),
        ))
        .routes(access::protected(
            Permission::ResourceWrite,
            routes!(resource::create_resource),
        ))
        .routes(access::protected(
            Permission::ResourceWrite,
            routes!(resource::update_resource),
        ))
        .routes(access::protected(
            Permission::ResourceDelete,
            routes!(resource::delete_resource),
        ))
        // Webhooks — registration producer (`mode=Prod`; mints secret + URL once).
        .routes(access::protected(
            Permission::WorkflowWrite,
            routes!(webhook::register_webhook),
        ))
        // Credentials (Plane B — API-owned OAuth flow). Literal paths first, then
        // collection, then parameterized `{cred}`, then sub-resources.
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(credential::resolve_credential),
        ))
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(credential::continue_resolve_credential),
        ))
        .routes(access::protected(
            Permission::CredentialRead,
            routes!(credential::list_credentials),
        ))
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(credential::create_credential),
        ))
        .routes(access::protected(
            Permission::CredentialRead,
            routes!(credential::get_credential),
        ))
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(credential::update_credential),
        ))
        .routes(access::protected(
            Permission::CredentialDelete,
            routes!(credential::delete_credential),
        ))
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(credential::get_oauth2_authorize_url_scoped),
        ))
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(
                credential::get_oauth2_callback_scoped,
                credential::post_oauth2_callback_scoped
            ),
        ))
        .routes(access::protected(
            Permission::CredentialRead,
            routes!(credential::test_credential),
        ))
        .routes(access::protected(
            Permission::CredentialWrite,
            routes!(credential::refresh_credential),
        ))
        .routes(access::protected(
            Permission::CredentialDelete,
            routes!(credential::revoke_credential),
        ))
}
