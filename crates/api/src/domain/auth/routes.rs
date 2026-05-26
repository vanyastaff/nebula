//! Authentication routes — split across an unauthenticated sub-router and
//! a session-bearing CSRF-gated sub-router.
//!
//! The session-gated subset (`/auth/mfa/enroll` + `/auth/mfa/verify`) is
//! mounted by `build_openapi_router` (in `crate::domain`) with both
//! `auth_middleware` and `csrf_middleware` layered. The rest of the
//! `/auth/*` surface stays on the flat unauthenticated path — including
//! the cookie-less second-factor login completion at `/auth/login/mfa`.

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// Flat `/auth/*` routes — mounted without `auth_middleware` or
/// `csrf_middleware`.
///
/// Two categories live here:
///
/// 1. Cookie-less / token-driven endpoints: `signup`, `login`,
///    `forgot-password`, `reset-password`, `verify-email`,
///    `mfa_complete_login` (login second step), `oauth_start`,
///    `oauth_callback`. None of them require a pre-existing session,
///    so neither layer applies.
///
/// 2. `logout` — *is* session-bearing (it revokes `nebula_session`
///    when present) but is intentionally kept CSRF-exempt. A CSRF
///    attack on logout can only force a sign-out (annoying, not a
///    confidentiality / integrity breach), and keeping the endpoint
///    reachable without a matching CSRF cookie makes it robust to
///    cookie-jar drift / clears.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handler::signup))
        .routes(routes!(handler::login))
        .routes(routes!(handler::logout))
        .routes(routes!(handler::forgot_password))
        .routes(routes!(handler::reset_password))
        .routes(routes!(handler::verify_email))
        .routes(routes!(handler::mfa_complete_login))
        .routes(routes!(handler::oauth_start))
        .routes(routes!(handler::oauth_callback))
}

/// Session-bearing `/auth/mfa/*` routes.
///
/// `mfa_enroll` and the enrollment-confirm `mfa_verify` both require an
/// authenticated session cookie. `build_openapi_router` (in
/// `crate::domain`) layers `auth_middleware` and `csrf_middleware` (in
/// that order) on this sub-router so the standard double-submit-cookie
/// contract applies.
pub fn mfa_session_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handler::mfa_enroll))
        .routes(routes!(handler::mfa_verify))
}
