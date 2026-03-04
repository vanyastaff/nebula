//! System/infra handlers.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use tracing::debug;

use crate::{
    models::{StatusResponse, WebhookStatus},
    state::ApiState,
};

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
