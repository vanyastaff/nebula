//! Organization-level routes — authenticated + org-scoped.
//!
//! Stub handlers under this group are marked `#[deprecated]` so the
//! generated OpenAPI spec flags them per ADR-0047 Stub Endpoint Policy.
//! The deprecation lint is silenced at module level — these handlers are
//! intentionally mounted today (returning 501) so the route table stays
//! in sync with the published spec.
#![allow(deprecated)]

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// Organization routes under `/api/v1/orgs/{org}/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            handlers::org::get_org,
            handlers::org::update_org,
            handlers::org::delete_org
        ))
        .routes(routes!(
            handlers::org::list_members,
            handlers::org::invite_member
        ))
        .routes(routes!(handlers::org::remove_member))
        .routes(routes!(
            handlers::org::list_service_accounts,
            handlers::org::create_service_account
        ))
        .routes(routes!(handlers::org::delete_service_account))
}
