//! Organization-level routes — authenticated + org-scoped.
//!
//! Stub handlers under this group are marked `#[deprecated]` so the
//! generated OpenAPI spec flags them per ADR-0047 Stub Endpoint Policy.
//! The deprecation lint is silenced at module level — these handlers are
//! intentionally mounted today (returning 501) so the route table stays
//! in sync with the published spec.
#![allow(deprecated)]

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// Organization routes under `/api/v1/orgs/{org}/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            handler::get_org,
            handler::update_org,
            handler::delete_org
        ))
        .routes(routes!(handler::list_members, handler::invite_member))
        .routes(routes!(handler::remove_member))
        .routes(routes!(
            handler::list_service_accounts,
            handler::create_service_account
        ))
        .routes(routes!(handler::delete_service_account))
}
