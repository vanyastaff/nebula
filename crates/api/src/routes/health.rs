//! Health check routes

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// Health routes (root level — `/health`, `/ready`, `/version`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::health::health_check))
        .routes(routes!(handlers::health::readiness_check))
        .routes(routes!(handlers::health::version_info))
}
