//! Router composition and middleware stack.

use axum::{
    BoxError, Router, error_handling::HandleErrorLayer, extract::DefaultBodyLimit, http::StatusCode,
};
use std::time::Duration;
use tower::{ServiceBuilder, timeout::TimeoutLayer};

use crate::{middleware::http_trace_layer, state::ApiState};

mod auth;
mod system;
mod workflows;

pub(crate) fn api_router() -> Router<ApiState> {
    let v1 = Router::new()
        .merge(system::v1_routes())
        .merge(auth::v1_routes())
        .merge(workflows::v1_routes());

    Router::new()
        .merge(system::public_routes())
        .merge(auth::oauth_routes())
        .nest("/api/v1", v1)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|_: BoxError| async {
                    StatusCode::REQUEST_TIMEOUT
                }))
                .layer(TimeoutLayer::new(Duration::from_secs(10)))
                .layer(http_trace_layer())
                .layer(auth::cors_layer()),
        )
}
