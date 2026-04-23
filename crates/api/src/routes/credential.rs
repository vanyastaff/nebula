//! System-level credential routes — not workspace-scoped.
//!
//! Type discovery endpoints expose the catalog of available credential
//! types and their schemas. OAuth2 callback routes handle external
//! provider redirects.

use axum::{Router, routing::get};

use crate::{handlers, state::AppState};

/// System-level credential endpoints under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        // Credential type discovery (system-wide catalog)
        .route(
            "/credentials/types",
            get(handlers::credential::list_credential_types),
        )
        .route(
            "/credentials/types/{key}",
            get(handlers::credential::get_credential_type),
        )
        // OAuth2 callback routes (external provider redirects, not workspace-scoped)
        .route(
            "/credentials/{id}/oauth2/auth",
            get(handlers::credential::get_oauth2_authorize_url),
        )
        .route(
            "/credentials/{id}/oauth2/callback",
            get(handlers::credential::get_oauth2_callback)
                .post(handlers::credential::post_oauth2_callback),
        )
}
