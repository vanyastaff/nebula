//! API router composition and server configuration.

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;
use std::sync::Arc;
use thiserror::Error;
use tracing::debug;

use crate::{
    auth::{auth_me, cors_layer, github_callback, oauth_callback, oauth_start},
    middleware::http_trace_layer,
    state::ApiState,
    status::{WebhookStatus, WorkerStatus},
    workflows::{create_workflow, delete_workflow, get_workflow, list_workflows, update_workflow},
};
use nebula_webhook::WebhookServer;

/// Configuration for the API server.
#[derive(Debug, Clone)]
pub struct ApiServerConfig {
    /// Bind address (e.g. `0.0.0.0:5678`).
    pub bind_addr: std::net::SocketAddr,
}

impl Default for ApiServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:5678".parse().unwrap(),
        }
    }
}

/// Response for `GET /api/v1/status`.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// Node workers (e.g. 4).
    pub workers: Vec<WorkerStatus>,
    /// Webhook server info.
    pub webhook: WebhookStatus,
}

/// API-only router (no webhook). Merge with `webhook_server.router()` for full app.
pub fn api_router() -> Router<ApiState> {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/status", get(status))
        .route("/auth/oauth/start", axum::routing::post(oauth_start))
        .route("/auth/oauth/callback", axum::routing::post(oauth_callback))
        .route("/auth/github/callback", get(github_callback))
        .route("/api/v1/auth/me", get(auth_me))
        .route(
            "/api/v1/workflows",
            get(list_workflows).post(create_workflow),
        )
        .route(
            "/api/v1/workflows/{id}",
            get(get_workflow)
                .patch(update_workflow)
                .delete(delete_workflow),
        )
        .layer(http_trace_layer())
        .layer(cors_layer())
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn status(State(state): State<ApiState>) -> impl IntoResponse {
    debug!("GET /api/v1/status");
    let webhook = WebhookStatus::from_server(state.webhook.as_ref());
    let response = StatusResponse {
        workers: state.workers.clone(),
        webhook,
    };
    Json(response)
}

/// Unified API server: holds config and can run the combined app.
pub struct ApiServer {
    #[allow(dead_code)] // reserved for future per-request config use
    config: ApiServerConfig,
}

impl ApiServer {
    /// Create with default config.
    pub fn new(config: ApiServerConfig) -> Self {
        Self { config }
    }

    /// Build the full app (API + webhook) for this server.
    pub fn app(&self, webhook_server: Arc<WebhookServer>, workers: Vec<WorkerStatus>) -> Router {
        crate::app(webhook_server, workers)
    }
}

/// Errors from the API server.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Webhook embedded creation failed.
    #[error("webhook: {0}")]
    Webhook(#[from] nebula_webhook::Error),
    /// IO (bind, serve).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
