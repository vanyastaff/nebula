//! User profile routes — authenticated, no tenant scope.
//!
//! Five handlers (`get_me`, `update_me`, `list_my_tokens`, `create_token`,
//! `delete_token`) are real end-to-end via the Plane-A `AuthBackend` port.
//! The one remaining stub — `list_my_orgs` — is `#[deprecated]` so the
//! generated OpenAPI spec flags it per ADR-0047 Stub Endpoint Policy
//! (canon §4.5 honest 501: principal→orgs enumeration is not wired until
//! the org/membership phase). The deprecation lint is silenced at module
//! level because the route table still references that one stub handler.
#![allow(deprecated)]

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
