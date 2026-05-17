//! System-level credential routes — not workspace-scoped.
//!
//! Type discovery endpoints expose the catalog of available credential
//! types and their schemas. OAuth2 callback routes handle external
//! provider redirects.

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// System-level credential endpoints under `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Credential type discovery (system-wide catalog)
        .routes(routes!(handler::list_credential_types))
        .routes(routes!(handler::get_credential_type))
        // OAuth2 callback routes (external provider redirects, not workspace-scoped)
        .routes(routes!(handler::get_oauth2_authorize_url))
        .routes(routes!(
            handler::get_oauth2_callback,
            handler::post_oauth2_callback
        ))
}
