//! OAuth credential HTTP ceremony (ADR-0031).
//!
//! Feature-gated behind `credential-oauth` during rollout.

pub mod flow;
pub mod oauth_controller;
pub mod state;

use axum::Router;

use crate::state::AppState;

/// Router for OAuth credential endpoints.
pub fn router() -> Router<AppState> {
    oauth_controller::router()
}
