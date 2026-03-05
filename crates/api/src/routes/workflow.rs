//! Workflow routes

use crate::handlers;
use crate::state::AppState;
use axum::{
    routing::get,
    Router,
};

/// Workflow routes
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/workflows", get(handlers::list_workflows).post(handlers::create_workflow))
        .route(
            "/workflows/{id}",
            get(handlers::get_workflow)
                .put(handlers::update_workflow)
                .delete(handlers::delete_workflow),
        )
}

