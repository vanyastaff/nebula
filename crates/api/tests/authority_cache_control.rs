//! Full-router cache-control coverage for responses that mint or rotate authority.

mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, Response, StatusCode, header},
};
use common::{
    TEST_CSRF_TOKEN, TEST_ORG, build_me_state, me_support::create_me_state,
    org_support::create_org_state_with_role,
};
use nebula_api::{
    ApiConfig, app,
    domain::auth::backend::{
        AuthBackend, CSRF_COOKIE, InMemoryAuthBackend, SESSION_COOKIE, SignupRequest,
        dto::SecretString, mfa,
    },
};
use nebula_core::OrgRole;
use serde_json::Value;
use tower::ServiceExt;

fn assert_no_store(response: &Response<Body>, route: &str) {
    let has_no_store = response
        .headers()
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"));

    assert!(
        has_no_store,
        "{route} returned authority without `Cache-Control: no-store`; headers: {:?}",
        response.headers()
    );
}

fn json_post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("JSON body")))
        .expect("valid request")
}

async fn register_user(
    backend: &InMemoryAuthBackend,
    email: &str,
) -> nebula_api::domain::auth::backend::UserProfile {
    backend
        .register_user(SignupRequest {
            email: email.to_owned(),
            password: SecretString::new("hunter22".to_owned()),
            display_name: "Authority Cache Test".to_owned(),
        })
        .await
        .expect("register user")
}

#[tokio::test]
async fn login_and_mfa_completion_authority_responses_are_no_store() {
    let backend = Arc::new(InMemoryAuthBackend::new());
    register_user(&backend, "plain-login@nebula.dev").await;
    let mfa_user = register_user(&backend, "mfa-login@nebula.dev").await;
    let enrollment = backend
        .start_mfa_enrollment(&mfa_user.user_id)
        .await
        .expect("start MFA enrollment");
    let enrollment_code =
        mfa::current_code(&enrollment.secret_base32).expect("current enrollment code");
    backend
        .confirm_mfa_enrollment(&mfa_user.user_id, &enrollment_code)
        .await
        .expect("confirm MFA enrollment");

    let backend_dyn: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
    let state = build_me_state().with_auth_backend(backend_dyn);
    let config = ApiConfig::for_test();

    let login = app::build_app(state.clone(), &config)
        .oneshot(json_post(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "plain-login@nebula.dev",
                "password": "hunter22"
            }),
        ))
        .await
        .expect("password login response");
    assert_eq!(login.status(), StatusCode::OK);
    assert_no_store(&login, "POST /api/v1/auth/login (200)");
    assert!(
        login.headers().contains_key(header::SET_COOKIE),
        "successful login must mint session cookies"
    );

    let challenge = app::build_app(state.clone(), &config)
        .oneshot(json_post(
            "/api/v1/auth/login",
            serde_json::json!({
                "email": "mfa-login@nebula.dev",
                "password": "hunter22"
            }),
        ))
        .await
        .expect("MFA challenge response");
    assert_eq!(challenge.status(), StatusCode::ACCEPTED);
    assert_no_store(&challenge, "POST /api/v1/auth/login (202)");
    let challenge_body = axum::body::to_bytes(challenge.into_body(), usize::MAX)
        .await
        .expect("MFA challenge body");
    let challenge_json: Value =
        serde_json::from_slice(&challenge_body).expect("MFA challenge JSON");
    let challenge_token = challenge_json["challenge_token"]
        .as_str()
        .expect("challenge token");

    let completion_code =
        mfa::current_code(&enrollment.secret_base32).expect("current completion code");
    let completion = app::build_app(state, &config)
        .oneshot(json_post(
            "/api/v1/auth/login/mfa",
            serde_json::json!({
                "challenge_token": challenge_token,
                "code": completion_code
            }),
        ))
        .await
        .expect("MFA completion response");
    assert_eq!(completion.status(), StatusCode::OK);
    assert_no_store(&completion, "POST /api/v1/auth/login/mfa");
    assert!(
        completion.headers().contains_key(header::SET_COOKIE),
        "MFA completion must mint session cookies"
    );
}

#[tokio::test]
async fn mfa_enrollment_seed_response_is_no_store() {
    let backend = Arc::new(InMemoryAuthBackend::new());
    let user = register_user(&backend, "mfa-enroll-cache@nebula.dev").await;
    let session = backend
        .create_session(&user.user_id)
        .await
        .expect("create fresh session");
    let backend_dyn: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
    let state = build_me_state().with_auth_backend(backend_dyn);
    let cookie = format!(
        "{SESSION_COOKIE}={}; {CSRF_COOKIE}={}",
        session.id, session.csrf_token
    );

    let response = app::build_app(state, &ApiConfig::for_test())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/mfa/enroll")
                .header(header::COOKIE, cookie)
                .header("x-csrf-token", &session.csrf_token)
                .body(Body::empty())
                .expect("MFA enrollment request"),
        )
        .await
        .expect("MFA enrollment response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_no_store(&response, "POST /api/v1/auth/mfa/enroll");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("MFA enrollment body");
    let json: Value = serde_json::from_slice(&body).expect("MFA enrollment JSON");
    assert!(
        json["secret_base32"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "MFA enrollment response must carry the one-time seed"
    );
}

#[tokio::test]
async fn pat_plaintext_response_is_no_store() {
    let (state, _backend, user) = create_me_state().await;
    let response = app::build_app(state, &ApiConfig::for_test())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/me/tokens")
                .header(header::AUTHORIZATION, format!("Bearer {}", user.jwt))
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::COOKIE,
                    format!("__Host-nebula-csrf={TEST_CSRF_TOKEN}"),
                )
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    r#"{"name":"cache-control","scopes":["full_access"]}"#,
                ))
                .expect("PAT create request"),
        )
        .await
        .expect("PAT create response");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_no_store(&response, "POST /api/v1/me/tokens");
}

#[tokio::test]
async fn service_account_key_route_is_preclassified_no_store() {
    let (state, _membership, owner) = create_org_state_with_role(OrgRole::OrgOwner);
    let route = format!("/api/v1/orgs/{TEST_ORG}/service-accounts");
    let response = app::build_app(state, &ApiConfig::for_test())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&route)
                .header(header::AUTHORIZATION, format!("Bearer {}", owner.jwt))
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::COOKIE,
                    format!("__Host-nebula-csrf={TEST_CSRF_TOKEN}"),
                )
                .header("x-csrf-token", TEST_CSRF_TOKEN)
                .body(Body::from(r#"{"name":"build-bot"}"#))
                .expect("service-account create request"),
        )
        .await
        .expect("service-account create response");

    assert_eq!(
        response.status(),
        StatusCode::NOT_IMPLEMENTED,
        "the route remains an honest 501 until service-account identity ships"
    );
    assert_no_store(&response, "POST /api/v1/orgs/{org}/service-accounts");
}
