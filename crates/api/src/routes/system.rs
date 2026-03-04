//! System route group.

use axum::{Router, routing::get};

use crate::{handlers::system, state::ApiState};

pub(super) fn public_routes() -> Router<ApiState> {
    Router::new()
        .route("/health", get(system::health))
        .route("/ready", get(system::ready))
}

pub(super) fn v1_routes() -> Router<ApiState> {
    Router::new().route("/status", get(system::status))
}
