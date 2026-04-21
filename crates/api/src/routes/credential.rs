//! OAuth credential routes.

use axum::Router;

use crate::state::AppState;

/// OAuth credential endpoints under `/api/v1`.
pub fn router() -> Router<AppState> {
    crate::credential::router()
}
