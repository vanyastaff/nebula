//! OpenAPI documentation routes — unauthenticated.

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// OpenAPI documentation routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::openapi::openapi_spec))
        .routes(routes!(handlers::openapi::docs_ui))
}
