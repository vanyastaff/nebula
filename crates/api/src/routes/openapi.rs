//! OpenAPI documentation routes — unauthenticated.

use axum::{Router, routing::get};

use crate::{handlers, state::AppState};

/// OpenAPI documentation routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/openapi.json", get(handlers::openapi::openapi_spec))
        .route("/docs", get(handlers::openapi::docs_ui))
}
