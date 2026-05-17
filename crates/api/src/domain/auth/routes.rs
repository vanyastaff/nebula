//! Authentication routes — unauthenticated endpoints.

use utoipa_axum::{router::OpenApiRouter, routes};

use super::handler;
use crate::state::AppState;

/// Auth routes under `/api/v1/auth/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handler::signup))
        .routes(routes!(handler::login))
        .routes(routes!(handler::logout))
        .routes(routes!(handler::forgot_password))
        .routes(routes!(handler::reset_password))
        .routes(routes!(handler::verify_email))
        .routes(routes!(handler::mfa_enroll))
        .routes(routes!(handler::mfa_verify))
        .routes(routes!(handler::oauth_start))
        .routes(routes!(handler::oauth_callback))
}
