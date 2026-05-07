//! Action and plugin catalog routes.

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// Action and plugin catalog routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::catalog::list_actions))
        .routes(routes!(handlers::catalog::get_action))
        .routes(routes!(handlers::catalog::list_plugins))
        .routes(routes!(handlers::catalog::get_plugin))
}
