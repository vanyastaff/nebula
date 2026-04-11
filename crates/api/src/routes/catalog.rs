//! Action and plugin catalog routes.

use axum::{Router, routing::get};

use crate::{handlers, state::AppState};

/// Action and plugin catalog routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/actions", get(handlers::list_actions))
        .route("/actions/{key}", get(handlers::get_action))
        .route("/plugins", get(handlers::list_plugins))
        .route("/plugins/{key}", get(handlers::get_plugin))
}
