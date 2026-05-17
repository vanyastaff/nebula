//! ADR-0052 P4 seam (V2): the credential write path validates `data`
//! against the credential type's schema **before persist**; rejection is
//! secret-safe (no submitted value echoed); an unconfigured port yields
//! 503 — `data` is never persisted unvalidated (closes the verified
//! §4.5/§10 fail-open).

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
    let app = app::build_app(state, &config);

    let body = serde_json::json!({
        "credential_key": "api_key",
        "name": "x",
        "data": { "api_key": "k", "leaky": NEVER_LEAK },
        "tags": {}
    });
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
}
