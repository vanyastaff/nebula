//! Authentication routes — unauthenticated endpoints.

use axum::{
    Router,
    routing::{get, post},
};

use crate::{handlers, state::AppState};

/// Auth routes under `/auth/*`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/signup", post(handlers::auth::signup))
        .route("/auth/login", post(handlers::auth::login))
        .route("/auth/logout", post(handlers::auth::logout))
        .route(
            "/auth/forgot-password",
            post(handlers::auth::forgot_password),
        )
        .route("/auth/reset-password", post(handlers::auth::reset_password))
        .route("/auth/verify-email", post(handlers::auth::verify_email))
        .route("/auth/mfa/enroll", post(handlers::auth::mfa_enroll))
        .route("/auth/mfa/verify", post(handlers::auth::mfa_verify))
        .route("/auth/oauth/{provider}", get(handlers::auth::oauth_start))
        .route(
            "/auth/oauth/{provider}/callback",
            get(handlers::auth::oauth_callback),
        )
}
