//! User profile routes — authenticated, no tenant scope.
//!
//! Stub handlers under this group are marked `#[deprecated]` so the
//! generated OpenAPI spec flags them per ADR-0047 Stub Endpoint Policy.
//! The deprecation lint is silenced at module level — these handlers are
//! intentionally mounted today (returning 501) so the route table stays
//! in sync with the published spec.
#![allow(deprecated)]

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// User profile routes under `/api/v1/me/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::me::get_me, handlers::me::update_me))
        .routes(routes!(handlers::me::list_my_orgs))
        .routes(routes!(
            handlers::me::list_my_tokens,
            handlers::me::create_token
        ))
        .routes(routes!(handlers::me::delete_token))
}
