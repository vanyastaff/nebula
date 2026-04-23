//! Health check routes

use axum::{Router, routing::get};

use crate::{handlers, state::AppState};

/// Health routes
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(handlers::health_check))
        .route("/ready", get(handlers::readiness_check))
        .route("/version", get(handlers::version_info))
}
