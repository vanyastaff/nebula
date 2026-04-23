//! Organization-level routes — authenticated + org-scoped.

use axum::{
    Router,
    routing::{delete, get},
};

use crate::{handlers, state::AppState};

/// Organization routes under `/orgs/{org}/*`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/orgs/{org}",
            get(handlers::org::get_org)
                .patch(handlers::org::update_org)
                .delete(handlers::org::delete_org),
        )
        .route(
            "/orgs/{org}/members",
            get(handlers::org::list_members).post(handlers::org::invite_member),
        )
        .route(
            "/orgs/{org}/members/{principal}",
            delete(handlers::org::remove_member),
        )
        .route(
            "/orgs/{org}/service-accounts",
            get(handlers::org::list_service_accounts).post(handlers::org::create_service_account),
        )
        .route(
            "/orgs/{org}/service-accounts/{sa}",
            delete(handlers::org::delete_service_account),
        )
}
