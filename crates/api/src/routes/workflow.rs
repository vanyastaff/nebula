//! Workflow routes

use crate::handlers;
use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};

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
}
