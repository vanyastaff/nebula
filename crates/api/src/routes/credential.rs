//! Integration credential routes (**Plane B** — ADR-0033).
//!
//! Mounted under `/api/v1` behind [`crate::middleware::auth`] (**Plane A**) so only authenticated
//! API clients start or complete OAuth flows for external integrations.

use axum::Router;

use crate::state::AppState;

/// OAuth integration-credential endpoints under `/api/v1`.
pub fn router() -> Router<AppState> {
    crate::credential::router()
}
