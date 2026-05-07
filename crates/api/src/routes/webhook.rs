//! Webhook trigger routes — special auth, no standard middleware.
//!
//! The route `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}` carries
//! per-trigger authentication; the standard JWT/API-key middleware
//! does NOT apply here. Authentication is enforced inside the
//! [`crate::webhook::WebhookDispatcher`] via the registered
//! [`crate::webhook::WebhookAuthConfig`] policy (HMAC, bearer token,
//! or none).
//!
//! The router attaches the dispatcher as an `axum::Extension` so the
//! handler can extract it without changes to the global `AppState`.
//! The default [`router`] mounts a fresh in-memory dispatcher with no
//! registrations — production wiring (a future task) registers
//! triggers from the storage layer at startup. Tests build their own
//! router via [`router_with_dispatcher`].

use std::sync::Arc;

use axum::{Extension, Router, routing::post};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState, webhook::WebhookDispatcher};

/// Webhook routes under `/api/v1/hooks/{org}/{ws}/{trigger_slug}` — used
/// by [`crate::routes::create_routes`] to populate the OpenAPI document.
///
/// Mounts an empty in-memory [`WebhookDispatcher`]. Until trigger
/// lifecycle wiring lands (out of scope for M3.3), every request
/// resolves to `404 Not Found` — the production-correct outcome for
/// "no registered trigger".
pub fn router() -> OpenApiRouter<AppState> {
    let dispatcher = Arc::new(WebhookDispatcher::new());
    OpenApiRouter::new()
        .routes(routes!(
            handlers::webhook::handle_webhook_post,
            handlers::webhook::handle_webhook_get
        ))
        .layer(Extension(dispatcher))
}

/// Build a webhook router with a caller-supplied dispatcher. Used by
/// integration tests that bypass the OpenAPI machinery — returns a plain
/// `axum::Router<AppState>` so callers can `.nest("/api/v1", router)` as
/// they did pre-ADR-0047 without touching `utoipa-axum` types.
pub fn router_with_dispatcher(dispatcher: Arc<WebhookDispatcher>) -> Router<AppState> {
    Router::new()
        .route(
            "/hooks/{org}/{ws}/{trigger_slug}",
            post(handlers::webhook::handle_webhook_post).get(handlers::webhook::handle_webhook_get),
        )
        .layer(Extension(dispatcher))
}
