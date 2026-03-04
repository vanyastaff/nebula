//! System/infra handlers.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::Serialize;
use tracing::debug;

use crate::{
    state::ApiState,
    status::{WebhookStatus, WorkerStatus},
};

/// Response for `GET /api/v1/status`.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// Node workers (e.g. 4).
    pub workers: Vec<WorkerStatus>,
    /// Webhook server info.
    pub webhook: WebhookStatus,
}

pub(crate) async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

pub(crate) async fn ready() -> impl IntoResponse {
    (StatusCode::OK, "READY")
}

pub(crate) async fn status(State(state): State<ApiState>) -> impl IntoResponse {
    debug!("GET /api/v1/status");
    let webhook = WebhookStatus::from_server(state.webhook.as_ref());
    let response = StatusResponse {
        workers: state.workers.clone(),
        webhook,
    };
    Json(response)
}
