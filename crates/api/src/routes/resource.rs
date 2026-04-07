//! Resource management routes.

use axum::{Router, routing::get};

use crate::handlers::resource;
use crate::state::AppState;

/// Resource routes nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new().route("/resources", get(resource::list_resources))
}
