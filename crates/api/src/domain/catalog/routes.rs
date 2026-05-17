//! Action and plugin catalog routes.

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// Action and plugin catalog routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handler::list_actions))
        .routes(routes!(handler::get_action))
        .routes(routes!(handler::list_plugins))
        .routes(routes!(handler::get_plugin))
}
