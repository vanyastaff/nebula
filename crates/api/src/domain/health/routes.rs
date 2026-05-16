//! Health check routes

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// Health routes (root level — `/health`, `/ready`, `/version`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handler::health_check))
        .routes(routes!(handler::readiness_check))
        .routes(routes!(handler::version_info))
}
