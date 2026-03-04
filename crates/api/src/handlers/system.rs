//! System/infra handlers.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use tracing::debug;

use crate::{models::StatusResponse, state::ApiState};

pub(crate) async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

pub(crate) async fn ready() -> impl IntoResponse {
    (StatusCode::OK, "READY")
}

pub(crate) async fn status(State(state): State<ApiState>) -> impl IntoResponse {
    debug!("GET /api/v1/status");
    let response = StatusResponse {
        workers: state.workers.clone(),
    };
    Json(response)
}
