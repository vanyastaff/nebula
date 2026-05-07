//! Authentication routes — unauthenticated endpoints.

use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{handlers, state::AppState};

/// Auth routes under `/api/v1/auth/*`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::auth::signup))
        .routes(routes!(handlers::auth::login))
        .routes(routes!(handlers::auth::logout))
        .routes(routes!(handlers::auth::forgot_password))
        .routes(routes!(handlers::auth::reset_password))
        .routes(routes!(handlers::auth::verify_email))
        .routes(routes!(handlers::auth::mfa_enroll))
        .routes(routes!(handlers::auth::mfa_verify))
        .routes(routes!(handlers::auth::oauth_start))
        .routes(routes!(handlers::auth::oauth_callback))
}
