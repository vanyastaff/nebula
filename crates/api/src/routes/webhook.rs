//! Webhook trigger routes — special auth, no standard middleware.

use axum::{Router, routing::post};

use crate::{handlers, state::AppState};

/// Webhook routes under `/hooks/{org}/{ws}/{trigger_slug}`.
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/hooks/{org}/{ws}/{trigger_slug}",
        post(handlers::webhook::handle_webhook_post).get(handlers::webhook::handle_webhook_get),
    )
}
