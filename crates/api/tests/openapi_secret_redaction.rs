//! Runtime redaction test for OpenAPI secret fields (M3.2 Task 3).
//!
//! Locks the redaction guarantee independent of the compile-time
//! `#[schema(write_only = true)]` annotation: even if the schema attribute is
//! removed in a refactor, the runtime path still cannot leak the plaintext
//! secret because [`SecretString`] does not implement `Serialize` and its
//! `Debug` impl redacts.
//!
//! The test boots a minimal axum router with one handler that takes a
//! `LoginRequest`, formats it via `Debug`, and returns the formatted text
//! as the response body. The test asserts the response body does **not**
//! contain the plaintext password.

use axum::{Json, Router, body::Body, http::Request, routing::post};
use nebula_api::domain::auth::backend::dto::LoginRequest;
use tower::ServiceExt;

const PLAINTEXT_PASSWORD: &str = "very-secret-hunter2-do-not-leak";

async fn echo_login_debug(Json(body): Json<LoginRequest>) -> String {
    format!("{body:?}")
}

#[tokio::test]
async fn login_request_debug_does_not_leak_password() {
    let app = Router::new().route("/test-login", post(echo_login_debug));

    let payload = serde_json::json!({
        "email": "user@example.com",
        "password": PLAINTEXT_PASSWORD,
    });

    let request = Request::builder()
        .method("POST")
        .uri("/test-login")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();

    let response = app.oneshot(request).await.expect("handler must respond");
    assert_eq!(response.status(), 200);

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("body must be readable");
    let body_str = std::str::from_utf8(&body_bytes).expect("body must be utf-8");

    assert!(
        !body_str.contains(PLAINTEXT_PASSWORD),
        "Debug output for LoginRequest leaked the plaintext password: {body_str}"
    );

    // Note: we deliberately do NOT assert any specific redaction
    // sentinel (e.g. `"***"`). The contract this test guards is
    // "plaintext does not leak through Debug output"; coupling to the
    // sentinel format would re-fail this test if `SecretString` ever
    // changes its private formatter to `[REDACTED]` etc., even though
    // the leak guarantee still holds. The exact sentinel is verified
    // separately in `auth::dto::tests::secret_string_debug_redacts`.
}

#[test]
fn openapi_spec_marks_login_password_write_only() {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(LoginRequest)))]
    struct TestDoc;

    let spec = TestDoc::openapi();
    let schema = spec
        .components
        .as_ref()
        .expect("components present")
        .schemas
        .get("LoginRequest")
        .expect("LoginRequest schema present");

    let json = serde_json::to_value(schema).expect("schema serializable");
    let password = json
        .pointer("/properties/password")
        .expect("LoginRequest.password property must exist in spec");

    assert_eq!(
        password.get("writeOnly"),
        Some(&serde_json::Value::Bool(true)),
        "LoginRequest.password must declare writeOnly = true (got: {password:#?})"
    );
    assert_eq!(
        password.get("format").and_then(|v| v.as_str()),
        Some("password"),
        "LoginRequest.password must declare format = 'password' (got: {password:#?})"
    );
    assert!(
        password.get("example").is_none(),
        "LoginRequest.password must NOT carry an example value (got: {password:#?})"
    );
}
