//! User profile routes — authenticated, no tenant scope.
//!
//! All six handlers are real end-to-end: `get_me` / `update_me` /
//! `list_my_tokens` / `create_token` / `delete_token` via the Plane-A
//! `AuthBackend` port, and `list_my_orgs` via the shared `MembershipStore`
//! principal→orgs enumeration (Phase 3 — the former honest-501 stub
//! graduated; no `#[deprecated]` handler remains in this module).

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// User profile routes under `/api/v1/me/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handler::get_me, handler::update_me))
        .routes(routes!(handler::list_my_orgs))
        .routes(routes!(handler::list_my_tokens, handler::create_token))
        .routes(routes!(handler::delete_token))
}
