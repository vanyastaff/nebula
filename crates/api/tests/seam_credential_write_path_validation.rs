//! credential-schema validation seam (V2): the credential write path validates `data`
//! against the credential type's schema **before persist**; rejection is
//! secret-safe (no submitted value echoed); an unconfigured port yields
//! 503 — `data` is never persisted unvalidated (closes the verified
//! honest capability/§10 fail-open).

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
    ports::credential_schema::{
        CredentialFieldError, CredentialSchemaPort, CredentialTypeDescriptor,
    },
};
use tower::ServiceExt;

/// Rejects any `data` that lacks an `api_key` field; accepts otherwise.
struct RequireApiKeyPort;

impl CredentialSchemaPort for RequireApiKeyPort {
    fn validate_data(
        &self,
        _credential_key: &str,
        data: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>> {
        if data.get("api_key").is_some() {
            Ok(())
        } else {
            Err(vec![CredentialFieldError {
                path: "/api_key".to_owned(),
                code: "required".to_owned(),
                message: "api_key is required".to_owned(),
            }])
        }
    }
    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        Vec::new()
    }
    fn get_type(&self, _k: &str) -> Option<CredentialTypeDescriptor> {
        None
    }
}

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
    let (state, _q) = create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(RequireApiKeyPort));
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
        text.contains("/api_key") && text.contains("required"),
        "422 body must carry the secret-safe path+code; got: {text}"
    );
    assert!(
        !text.contains(NEVER_LEAK),
        "422 body must NOT echo any submitted value (secret-safe); got: {text}"
    );
}

#[tokio::test]
async fn create_credential_persists_valid_data() {
    let (state, _q) = create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(RequireApiKeyPort));
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
async fn create_credential_503_when_port_unconfigured_never_persists() {
    let (state, _q) = create_state_with_queue_no_credential_port().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "never-persist-probe",
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
        StatusCode::SERVICE_UNAVAILABLE,
        "no credential-schema port ⇒ 503 (never persist unvalidated)"
    );
    let text = body_string(resp).await;
    assert!(
        !text.contains(NEVER_LEAK),
        "503 body must not echo submitted data; got: {text}"
    );

    // Prove the "never persists" guarantee (not just the 503 status):
    // the rejected create must have written nothing to the store.
    let app = app::build_app(state, &config);
    let list = app
        .oneshot(auth_get(&ws_path("/credentials"), &token))
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK, "list must be reachable");
    let listed = body_string(list).await;
    assert!(
        !listed.contains("never-persist-probe") && !listed.contains(NEVER_LEAK),
        "rejected create must persist nothing — credential absent from list; got: {listed}"
    );
}

// ── CSRF wiring on credential write paths (M3.1 box 2) ────────────────────
//
// `csrf_middleware` is layered on `credential_routes` in `domain/mod.rs`.
// JWT auth is a cookie-bearing auth method, so the double-submit
// `X-CSRF-Token` header MUST match the `nebula_csrf` cookie for any
// state-changing request to reach the handler.

fn json_with_csrf_headers(
    method: &str,
    uri: &str,
    token: &str,
    body: &serde_json::Value,
    csrf_header: Option<&str>,
    csrf_cookie: Option<&str>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"));
    if let Some(h) = csrf_header {
        b = b.header("x-csrf-token", h);
    }
    if let Some(c) = csrf_cookie {
        b = b.header("cookie", c);
    }
    b.body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn create_credential_returns_403_when_csrf_header_missing() {
    // Cookie-bearing JWT auth + state-changing method + missing CSRF header
    // ⇒ csrf_middleware rejects with 403 *before* the handler runs (and
    // therefore *before* any credential-schema validation).
    let (state, _q) = create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(RequireApiKeyPort));
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
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
            &token,
            &body,
            None,
            Some(TEST_CSRF_COOKIE),
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
    let (state, _q) = create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(RequireApiKeyPort));
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
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
            &token,
            &body,
            Some("different-from-cookie"),
            Some(TEST_CSRF_COOKIE),
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
    // credential-schema validation: the V2 gate also covers the update path (validates the
    // supplied `data` against the *existing* credential type's schema).
    let (state, _q) = create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(RequireApiKeyPort));
    let config = ApiConfig::for_test();
    let token = create_test_jwt();

    // Seed a valid credential (RequireApiKeyPort accepts `api_key`).
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
        text.contains("/api_key") && text.contains("required") && !text.contains(NEVER_LEAK),
        "update 400 must carry path+code and echo no value (symmetric with the \
         create-path contract); got: {text}"
    );
}
