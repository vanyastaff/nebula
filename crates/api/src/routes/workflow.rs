//! Workflow routes

use axum::{
    Router,
    routing::{get, post},
};

use crate::{handlers, state::AppState};

/// Workflow routes
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/workflows",
            get(handlers::list_workflows).post(handlers::create_workflow),
        )
        .route(
            "/workflows/{id}",
            get(handlers::get_workflow)
                .put(handlers::update_workflow)
                .delete(handlers::delete_workflow),
        )
        .route(
            "/workflows/{id}/activate",
            post(handlers::activate_workflow),
        )
        .route("/workflows/{id}/execute", post(handlers::execute_workflow))
        .route(
            "/workflows/{id}/validate",
            post(handlers::validate_workflow_handler),
        )
}
