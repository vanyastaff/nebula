//! Execution routes

use crate::handlers;
use crate::state::AppState;
use axum::{
    routing::{get, post},
    Router,
};

/// Execution routes
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/workflows/{workflow_id}/executions",
            get(handlers::list_executions).post(handlers::start_execution),
        )
        .route(
            "/executions/{id}",
            get(handlers::get_execution),
        )
        .route(
            "/executions/{id}/cancel",
            post(handlers::cancel_execution),
        )
}

