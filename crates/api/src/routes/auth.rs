//! Auth route groups.

use axum::{Router, routing::get};

use crate::{
    auth::{auth_me, github_callback, oauth_callback, oauth_start},
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
