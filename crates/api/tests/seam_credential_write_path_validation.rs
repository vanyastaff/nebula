//! Canonical credential write validation: authenticated commands cross the
//! gateway first, then the credential-owned controller/service validates
//! exactly once before persistence. The API catalog port is not a competing
//! mutation validator.

mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN, create_state_with_queue,
    create_state_with_queue_no_credential_port, create_test_jwt, ws_path,
};
use nebula_api::{
    ApiConfig, app,
    domain::auth::backend::{AuthBackend, InMemoryAuthBackend, SignupRequest, dto::SecretString},
};
use tower::ServiceExt;

fn auth_json(method: &str, uri: &str, token: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

fn auth_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .body(Body::empty())
        .unwrap()
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

const NEVER_LEAK: &str = "SUPERSECRET-must-never-surface-0xDEADBEEF";

#[tokio::test]
async fn create_credential_rejects_invalid_data_secret_safe() {
    let (state, _q) = create_state_with_queue_no_credential_port().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
    let app = app::build_app(state, &config);

    // No `api_key` → rejected; a secret-bearing sibling field MUST NOT echo.
    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "bad",
        "data": { "server": "https://x", "leaky": NEVER_LEAK },
        "tags": {}
    });
    let resp = app
        .oneshot(auth_json("POST", &ws_path("/credentials"), &token, &body))
        .await
        .unwrap();
    // `ApiError::Validation` is the api-wide validation class → 400
    // (consistent with every other request-validation failure).
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "invalid credential data must be rejected with the api validation status (400)"
    );
    let text = body_string(resp).await;
    assert!(
        text.contains("/data") && text.contains("required"),
        "400 body must carry the API-owned coarse path+code; got: {text}"
    );
    assert!(
        !text.contains(NEVER_LEAK),
        "400 body must NOT echo any submitted value (secret-safe); got: {text}"
    );
}

#[tokio::test]
async fn create_credential_persists_valid_data() {
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
    let app = app::build_app(state, &config);

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "good",
        "data": { "api_key": "k-123" },
        "tags": {}
    });
    let resp = app
        .oneshot(auth_json("POST", &ws_path("/credentials"), &token, &body))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "valid credential data must persist (200)"
    );
}

#[tokio::test]
async fn create_credential_uses_canonical_validation_without_catalog_port() {
    let (state, _q) = create_state_with_queue_no_credential_port().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "canonical-validation-probe",
        "data": { "api_key": "k", "leaky": NEVER_LEAK },
        "tags": {}
    });
    let app = app::build_app(state.clone(), &config);
    let resp = app
        .oneshot(auth_json("POST", &ws_path("/credentials"), &token, &body))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "catalog availability must not control the authenticated mutation path"
    );
    let text = body_string(resp).await;
    assert!(
        !text.contains(NEVER_LEAK),
        "successful response must not echo submitted data; got: {text}"
    );

    // The credential-owned service validated and persisted the command even
    // though type discovery is unavailable.
    let app = app::build_app(state, &config);
    let list = app
        .oneshot(auth_get(&ws_path("/credentials"), &token))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK, "list must be reachable");
    let listed = body_string(list).await;
    assert!(
        listed.contains("canonical-validation-probe") && !listed.contains(NEVER_LEAK),
        "canonical validation must persist only the secret-free head; got: {listed}"
    );
}

// ── CSRF wiring on credential write paths (M3.1 box 2) ────────────────────
//
// `csrf_middleware` is layered on `credential_routes` in `domain/mod.rs`.
// Only ambient session-cookie authentication needs the double-submit proof;
// explicit Bearer/JWT authority is deliberately exempt. These tests therefore
// create a real session and prove that `X-CSRF-Token` MUST match the
// `__Host-nebula-csrf` cookie before a state-changing request reaches the
// handler.

async fn session_authenticated_credential_state() -> (nebula_api::AppState, String, String) {
    let (state, _q) = create_state_with_queue().await;
    let backend = Arc::new(InMemoryAuthBackend::new());
    let profile = backend
        .register_user(SignupRequest {
            email: "credential-csrf@nebula.dev".to_owned(),
            password: SecretString::new("hunter22".to_owned()),
            display_name: "Credential CSRF".to_owned(),
        })
        .await
        .expect("register session-authenticated user");
    let mut session = backend
        .create_session(&profile.user_id)
        .await
        .expect("create session authority");
    let session_id = std::mem::take(&mut session.id);
    let csrf_token = std::mem::take(&mut session.csrf_token);
    let backend_dyn: Arc<dyn AuthBackend> = backend;
    (state.with_auth_backend(backend_dyn), session_id, csrf_token)
}

fn json_with_csrf_headers(
    method: &str,
    uri: &str,
    body: &serde_json::Value,
    csrf_header: Option<&str>,
    session_cookie: &str,
    csrf_cookie: &str,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header(
            "cookie",
            format!("__Host-nebula-session={session_cookie}; __Host-nebula-csrf={csrf_cookie}"),
        );
    if let Some(h) = csrf_header {
        b = b.header("x-csrf-token", h);
    }
    b.body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn create_credential_returns_403_when_csrf_header_missing() {
    // Ambient session auth + state-changing method + missing CSRF header
    // ⇒ csrf_middleware rejects with 403 *before* the handler runs (and
    // therefore *before* any credential-schema validation).
    let (state, session_id, csrf_token) = session_authenticated_credential_state().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state, &config);

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "csrf-missing",
        "data": { "api_key": "k-1" },
        "tags": {}
    });
    let resp = app
        .oneshot(json_with_csrf_headers(
            "POST",
            &ws_path("/credentials"),
            &body,
            None,
            &session_id,
            &csrf_token,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "missing X-CSRF-Token on a cookie-auth write must be rejected by csrf_middleware"
    );
    let text = body_string(resp).await;
    assert!(
        text.to_lowercase().contains("csrf"),
        "403 body should mention CSRF; got: {text}"
    );
}

#[tokio::test]
async fn create_credential_returns_403_when_csrf_header_mismatches_cookie() {
    let (state, session_id, csrf_token) = session_authenticated_credential_state().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state, &config);

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "csrf-mismatch",
        "data": { "api_key": "k-2" },
        "tags": {}
    });
    let resp = app
        .oneshot(json_with_csrf_headers(
            "POST",
            &ws_path("/credentials"),
            &body,
            Some("different-from-cookie"),
            &session_id,
            &csrf_token,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "X-CSRF-Token / cookie mismatch on a cookie-auth write must be rejected"
    );
    let text = body_string(resp).await;
    assert!(
        text.to_lowercase().contains("csrf"),
        "403 body should mention CSRF; got: {text}"
    );
}

#[tokio::test]
async fn update_credential_rejects_invalid_data_secret_safe() {
    // The credential-owned service validates supplied `data` against the
    // existing credential type under the same authority decision as the write.
    let (state, _q) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a valid credential through the same canonical service path.
    let app = app::build_app(state.clone(), &config);
    let created = app
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials"),
            &token,
            &serde_json::json!({
                "credential_key": "api_key", "name": "u",
                "data": { "api_key": "k-1" }, "tags": {}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK, "seed create must succeed");
    let id = serde_json::from_str::<serde_json::Value>(&body_string(created).await).unwrap()["id"]
        .as_str()
        .expect("created id")
        .to_owned();

    // PUT new `data` that fails the schema (no `api_key`) + a secret.
    let app = app::build_app(state, &config);
    let resp = app
        .oneshot(auth_json(
            "PUT",
            &ws_path(&format!("/credentials/{id}")),
            &token,
            &serde_json::json!({ "data": { "server": "x", "leak": NEVER_LEAK } }),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "update with schema-invalid data must be rejected (400)"
    );
    let text = body_string(resp).await;
    assert!(
        text.contains("/data") && text.contains("required") && !text.contains(NEVER_LEAK),
        "update 400 must carry path+code and echo no value (symmetric with the \
         create-path contract); got: {text}"
    );
}
