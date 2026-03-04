//! Auth route groups.

use axum::{Router, routing::get};

use crate::{
    auth::{auth_me, cors_layer as auth_cors_layer, github_callback, oauth_callback, oauth_start},
    state::ApiState,
};

pub(super) fn oauth_routes() -> Router<ApiState> {
    Router::new()
        .route("/auth/oauth/start", axum::routing::post(oauth_start))
        .route("/auth/oauth/callback", axum::routing::post(oauth_callback))
        .route("/auth/github/callback", get(github_callback))
}

pub(super) fn v1_routes() -> Router<ApiState> {
    Router::new().route("/auth/me", get(auth_me))
}

pub(super) fn cors_layer() -> tower_http::cors::CorsLayer {
    auth_cors_layer()
}
