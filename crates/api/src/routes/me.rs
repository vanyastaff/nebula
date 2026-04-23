//! User profile routes — authenticated, no tenant scope.

use axum::{
    Router,
    routing::{delete, get},
};

use crate::{handlers, state::AppState};

/// User profile routes under `/me/*`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/me",
            get(handlers::me::get_me).patch(handlers::me::update_me),
        )
        .route("/me/orgs", get(handlers::me::list_my_orgs))
        .route(
            "/me/tokens",
            get(handlers::me::list_my_tokens).post(handlers::me::create_token),
        )
        .route("/me/tokens/{pat}", delete(handlers::me::delete_token))
}
