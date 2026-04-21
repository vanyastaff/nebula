//! Integration credential HTTP adapters — **Plane B** (ADR-0033) / OAuth ceremony (ADR-0031).
//!
//! HTTP endpoints that implement *acquisition* for [`nebula_credential::Credential`] types
//! (e.g. OAuth2 authorize URL + callback). They delegate to the engine/credential pipeline; they do
//! **not** define `Credential` semantics and are **not** Nebula operator login (**Plane A** —
//! see [`crate::middleware::auth`]).
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
