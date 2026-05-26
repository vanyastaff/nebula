//! CSRF wiring on `/auth/mfa/*` (M3.1 box 2).
//!
//! `mfa_enroll` and `mfa_verify` (enrollment-confirm) both require a
//! session cookie and are mounted on a session-bearing sub-router that
//! layers `auth_middleware` followed by `csrf_middleware` (per
//! `crate::domain::build_openapi_router`). These tests probe the
//! enforcement contract:
//!
//! 1. A session-bearing request without the matching `X-CSRF-Token`
//!    header is rejected with 403 by `csrf_middleware` **before** the
//!    handler runs.
//! 2. The cookie-less second-factor login completion at
//!    `POST /auth/login/mfa` is CSRF-exempt by construction (the caller
//!    has no session yet, so the double-submit-cookie contract is
//!    irrelevant). The endpoint reaches its handler even without the
//!    `X-CSRF-Token` header.

mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::build_me_state;
use nebula_api::{
    ApiConfig, AppState, app,
    domain::auth::backend::{AuthBackend, InMemoryAuthBackend, SignupRequest, dto::SecretString},
};
use tower::ServiceExt;

/// Helper: register a real user against an `InMemoryAuthBackend`, mint a
/// session cookie pair, and return the wired `AppState`, the session id,
/// and the matching CSRF token.
async fn session_state() -> (AppState, String, String) {
    let backend = Arc::new(InMemoryAuthBackend::new());
    let profile = backend
        .register_user(SignupRequest {
            email: "mfa-csrf@nebula.dev".to_owned(),
            password: SecretString::new("hunter22".to_owned()),
            display_name: "Mfa CSRF".to_owned(),
        })
        .await
        .expect("register user");
    let session = backend
        .create_session(&profile.user_id)
        .await
        .expect("create session");
    let backend_dyn: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
    let state = build_me_state().with_auth_backend(backend_dyn);
    (state, session.id, session.csrf_token)
}

fn session_cookie_pair(session_id: &str, csrf_value: &str) -> String {
    format!("nebula_session={session_id}; nebula_csrf={csrf_value}")
}

// ── 1. enroll: missing CSRF header on session-bearing request → 403 ──────────

#[tokio::test]
async fn mfa_enroll_returns_403_when_csrf_header_missing_with_session() {
    let (state, session_id, csrf_value) = session_state().await;
    let app = app::build_app(state, &ApiConfig::for_test());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/mfa/enroll")
                .header("cookie", session_cookie_pair(&session_id, &csrf_value))
                // Intentionally NO `x-csrf-token` header.
                .body(Body::empty())
                .expect("enroll request"),
        )
        .await
        .expect("enroll response");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "missing X-CSRF-Token on a session-bearing /auth/mfa/enroll must be \
         rejected by csrf_middleware"
    );
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8_lossy(&body).into_owned();
    assert!(
        text.to_lowercase().contains("csrf"),
        "403 body should mention CSRF; got: {text}"
    );
}

// ── 2. verify enrollment-confirm path: missing CSRF on session → 403 ─────────

#[tokio::test]
async fn mfa_verify_enroll_path_returns_403_when_csrf_header_missing() {
    let (state, session_id, csrf_value) = session_state().await;
    let app = app::build_app(state, &ApiConfig::for_test());

    let body = serde_json::json!({ "code": "123456" });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/mfa/verify")
                .header("content-type", "application/json")
                .header("cookie", session_cookie_pair(&session_id, &csrf_value))
                // Intentionally NO `x-csrf-token` header.
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .expect("verify request"),
        )
        .await
        .expect("verify response");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "missing X-CSRF-Token on a session-bearing /auth/mfa/verify must be \
         rejected by csrf_middleware"
    );
}

// ── 3. login second-step: cookie-less endpoint reaches handler w/o CSRF ──────

#[tokio::test]
async fn mfa_complete_login_succeeds_without_csrf_header() {
    // Cookie-less endpoint: no session cookie, no CSRF cookie/header.
    // The request MUST reach the handler (which then returns 401 because
    // the challenge token is invalid). Reaching the handler at all
    // proves the route is CSRF-exempt by construction.
    let backend = Arc::new(InMemoryAuthBackend::new());
    let backend_dyn: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
    let state = build_me_state().with_auth_backend(backend_dyn);
    let app = app::build_app(state, &ApiConfig::for_test());

    let body = serde_json::json!({
        "code": "123456",
        "challenge_token": "made-up-challenge"
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login/mfa")
                .header("content-type", "application/json")
                // NO session cookie, NO csrf header — the route is
                // cookie-less and CSRF-exempt.
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .expect("login mfa request"),
        )
        .await
        .expect("login mfa response");

    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "cookie-less /auth/login/mfa must not be rejected by csrf_middleware; \
         got 403 which means CSRF wiring leaked onto a cookie-less route"
    );
    // Concretely: invalid challenge token => 401 from the backend.
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "invalid challenge token must reach the handler and return 401"
    );
}
