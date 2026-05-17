//! Organization-level routes — authenticated + org-scoped.
//!
//! The **member** routes (`list_members` / `add_member` / `remove_member`)
//! are live (real 200/201 + typed errors). The **org-record** and
//! **service-account** routes are still honest-501 stubs marked
//! `#[deprecated]` so the generated OpenAPI spec flags them per ADR-0047
//! Stub Endpoint Policy. The deprecation lint is silenced at module level
//! because the stub handlers are intentionally mounted (returning 501) so
//! the route table stays in sync with the published spec — the
//! non-deprecated member handlers are unaffected by the allow.
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
        .routes(routes!(handler::list_members, handler::add_member))
        .routes(routes!(handler::remove_member))
        .routes(routes!(
            handler::list_service_accounts,
            handler::create_service_account
        ))
        .routes(routes!(handler::delete_service_account))
}
