//! Health check routes

use crate::handlers;
use crate::state::AppState;
use axum::{Router, routing::get};

/// Health routes
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(handlers::health_check))
        .route("/ready", get(handlers::readiness_check))
}
