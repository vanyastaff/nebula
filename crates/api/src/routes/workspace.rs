//! Workspace-scoped routes — authenticated + tenant-scoped.
//!
//! All routes under `/orgs/{org}/workspaces/{ws}/*` are behind
//! auth + tenancy + RBAC middleware layers.

use axum::{
    Router,
    routing::{get, post},
};

use crate::{handlers, state::AppState};

/// Workspace-scoped routes.
pub fn router() -> Router<AppState> {
    let router = Router::new()
        // Workflows
        .route(
            "/orgs/{org}/workspaces/{ws}/workflows",
            get(handlers::workflow::list_workflows)
                .post(handlers::workflow::create_workflow),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/workflows/{wf}",
            get(handlers::workflow::get_workflow)
                .put(handlers::workflow::update_workflow)
                .delete(handlers::workflow::delete_workflow),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/workflows/{wf}/activate",
            post(handlers::workflow::activate_workflow),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/workflows/{wf}/execute",
            post(handlers::workflow::execute_workflow),
        )
        // Executions
        .route(
            "/orgs/{org}/workspaces/{ws}/workflows/{wf}/executions",
            get(handlers::execution::list_executions_for_workflow)
                .post(handlers::execution::start_execution),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/executions",
            get(handlers::execution::list_executions),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/executions/{exec}",
            get(handlers::execution::get_execution)
                .delete(handlers::execution::cancel_execution),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/executions/{exec}/terminate",
            post(handlers::execution::terminate_execution),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/executions/{exec}/restart",
            post(handlers::execution::restart_execution),
        )
        // Resources
        .route(
            "/orgs/{org}/workspaces/{ws}/resources",
            get(handlers::resource::list_resources),
        );

    // Credentials (Plane B — ADR-0031).
    //
    // Route order: literal segments first (`resolve`, `resolve/continue`),
    // then collection, then parameterized `{cred}`, then sub-resources.
    router
        // ── Credential acquisition (literal paths before {cred}) ────
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials/resolve",
            post(handlers::credential::resolve_credential),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials/resolve/continue",
            post(handlers::credential::continue_resolve_credential),
        )
        // ── Credential CRUD ─────────────────────────────────────────
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials",
            get(handlers::credential::list_credentials)
                .post(handlers::credential::create_credential),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials/{cred}",
            get(handlers::credential::get_credential)
                .put(handlers::credential::update_credential)
                .delete(handlers::credential::delete_credential),
        )
        // ── Credential lifecycle ────────────────────────────────────
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials/{cred}/test",
            post(handlers::credential::test_credential),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials/{cred}/refresh",
            post(handlers::credential::refresh_credential),
        )
        .route(
            "/orgs/{org}/workspaces/{ws}/credentials/{cred}/revoke",
            post(handlers::credential::revoke_credential),
        )
}
