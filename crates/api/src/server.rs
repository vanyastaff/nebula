//! API server and routes.

use axum::{
    Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::debug;
use uuid::Uuid;

use crate::status::{WebhookStatus, WorkerStatus};
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

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Embedded webhook server (same process).
    pub webhook: Arc<WebhookServer>,
    /// Snapshot of node workers (e.g. 4 workers).
    pub workers: Vec<WorkerStatus>,
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
        .route("/auth/oauth/start", post(oauth_start))
        .route("/auth/oauth/callback", post(oauth_callback))
        .layer(api_cors_layer())
}

fn api_cors_layer() -> CorsLayer {
    // Optional override: comma-separated origins.
    // Example:
    // NEBULA_CORS_ALLOW_ORIGINS=http://localhost:5173,tauri://localhost
    let configured = std::env::var("NEBULA_CORS_ALLOW_ORIGINS")
        .ok()
        .unwrap_or_default();

    let mut origins: Vec<axum::http::HeaderValue> = configured
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| axum::http::HeaderValue::from_str(value).ok())
        .collect();

    if origins.is_empty() {
        origins = vec![
            axum::http::HeaderValue::from_static("http://localhost:5173"),
            axum::http::HeaderValue::from_static("http://127.0.0.1:5173"),
            axum::http::HeaderValue::from_static("http://tauri.localhost"),
            axum::http::HeaderValue::from_static("tauri://localhost"),
        ];
    }

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any)
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthStartRequest {
    provider: String,
    redirect_uri: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthStartResponse {
    auth_url: String,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthCallbackRequest {
    provider: String,
    code: String,
    redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthCallbackResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

fn is_mock_oauth_enabled() -> bool {
    std::env::var("NEBULA_OAUTH_MOCK")
        .ok()
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(true)
}

fn is_supported_provider(provider: &str) -> bool {
    matches!(provider, "google" | "github")
}

async fn oauth_start(Json(req): Json<OAuthStartRequest>) -> impl IntoResponse {
    let provider = req.provider.to_lowercase();
    if !is_supported_provider(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_provider",
                "message": "provider must be one of: google, github"
            })),
        )
            .into_response();
    }

    let state = Uuid::new_v4().to_string();

    if is_mock_oauth_enabled() {
        // Dev scaffold: emulate provider redirect directly back to desktop deep-link.
        let auth_url = format!(
            "{}?code=mock_{}&provider={}&state={}",
            req.redirect_uri, state, provider, state
        );
        return (
            StatusCode::OK,
            Json(serde_json::json!(OAuthStartResponse { auth_url, state })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "oauth_not_configured",
            "message": "real OAuth provider config is not implemented yet",
            "provider": provider
        })),
    )
        .into_response()
}

async fn oauth_callback(Json(req): Json<OAuthCallbackRequest>) -> impl IntoResponse {
    let provider = req.provider.to_lowercase();
    if !is_supported_provider(&provider) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "unsupported_provider",
                "message": "provider must be one of: google, github"
            })),
        )
            .into_response();
    }

    if req.code.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid_code",
                "message": "code is required"
            })),
        )
            .into_response();
    }

    if is_mock_oauth_enabled() && req.code.starts_with("mock_") {
        let token = format!("mock_token_{}_{}", provider, Uuid::new_v4());
        return (
            StatusCode::OK,
            Json(serde_json::json!(OAuthCallbackResponse {
                access_token: token,
                token_type: "Bearer".to_string(),
                expires_in: 3600
            })),
        )
            .into_response();
    }

    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": "oauth_callback_not_configured",
            "message": "real OAuth code exchange is not implemented yet",
            "provider": provider,
            "redirectUri": req.redirect_uri
        })),
    )
        .into_response()
}

/// Unified API server: holds config and can run the combined app.
pub struct ApiServer {
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
