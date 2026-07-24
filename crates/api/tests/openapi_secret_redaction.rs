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
use nebula_api::domain::{
    auth::backend::dto::{
        LoginRequest, LoginResponse, MfaChallengeResponse, MfaConfirmEnrollRequest,
        MfaEnrollResponse, MfaLoginCompleteRequest, OAuthStartResponse, ResetPasswordRequest,
        VerifyEmailRequest,
    },
    credential::dto::{
        AcquisitionInteraction, ContinueResolveRequest, CreateCredentialRequest, FormPostField,
        ResolveCredentialRequest, ResolveCredentialResponse, UpdateCredentialRequest,
    },
    me::dto::CreateTokenResponse,
    org::dto::CreateServiceAccountResponse,
    webhook::dto::{RegisterWebhookRequest, RegisterWebhookResponse},
};
use tower::ServiceExt;

const PLAINTEXT_PASSWORD: &str = "very-secret-hunter2-do-not-leak";

async fn echo_login_debug(Json(body): Json<LoginRequest>) -> String {
    format!("{body:?}")
}

#[test]
fn openapi_login_response_keeps_session_bearer_cookie_only() {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(LoginResponse)))]
    struct TestDoc;

    let spec = serde_json::to_value(TestDoc::openapi()).expect("schema serializes");
    let properties = spec
        .pointer("/components/schemas/LoginResponse/properties")
        .and_then(serde_json::Value::as_object)
        .expect("LoginResponse properties must exist");

    assert!(
        !properties.contains_key("session_id"),
        "the HttpOnly session bearer must never be a JSON response property"
    );
    assert_eq!(
        properties
            .get("csrf_token")
            .and_then(|property| property.get("readOnly")),
        Some(&serde_json::Value::Bool(true))
    );
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

#[test]
fn openapi_spec_marks_one_time_generated_credentials_read_only() {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(CreateTokenResponse, CreateServiceAccountResponse)))]
    struct TestDoc;

    let spec = serde_json::to_value(TestDoc::openapi()).expect("schema serializes");
    for (schema, property) in [
        ("CreateTokenResponse", "token"),
        ("CreateServiceAccountResponse", "key"),
    ] {
        let pointer = format!("/components/schemas/{schema}/properties/{property}");
        let credential = spec
            .pointer(&pointer)
            .unwrap_or_else(|| panic!("{schema}.{property} property must exist"));

        assert_eq!(
            credential.get("readOnly"),
            Some(&serde_json::Value::Bool(true)),
            "{schema}.{property} must declare readOnly = true: {credential:#?}"
        );
        assert!(
            credential.get("writeOnly").is_none(),
            "{schema}.{property} must not declare writeOnly: {credential:#?}"
        );
        assert_eq!(
            credential.get("format").and_then(serde_json::Value::as_str),
            Some("password")
        );
    }
}

fn assert_schema_property_flag(spec: &serde_json::Value, schema: &str, property: &str, flag: &str) {
    let pointer = format!("/components/schemas/{schema}/properties/{property}");
    let value = spec
        .pointer(&pointer)
        .unwrap_or_else(|| panic!("{schema}.{property} property must exist"));
    assert_eq!(
        value.get(flag),
        Some(&serde_json::Value::Bool(true)),
        "{schema}.{property} must declare {flag}=true: {value:#?}"
    );
}

fn collect_property_occurrences<'a>(
    value: &'a serde_json::Value,
    property: &str,
    found: &mut Vec<&'a serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(candidate) = object
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .and_then(|properties| properties.get(property))
            {
                found.push(candidate);
            }
            for child in object.values() {
                collect_property_occurrences(child, property, found);
            }
        },
        serde_json::Value::Array(array) => {
            for child in array {
                collect_property_occurrences(child, property, found);
            }
        },
        _ => {},
    }
}

#[test]
fn openapi_auth_authority_has_explicit_direction() {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(
        LoginRequest,
        ResetPasswordRequest,
        VerifyEmailRequest,
        MfaConfirmEnrollRequest,
        MfaLoginCompleteRequest,
        MfaChallengeResponse,
        MfaEnrollResponse,
        OAuthStartResponse
    )))]
    struct TestDoc;

    let spec = serde_json::to_value(TestDoc::openapi()).expect("schema serializes");
    for (schema, property) in [
        ("LoginRequest", "totp"),
        ("ResetPasswordRequest", "token"),
        ("VerifyEmailRequest", "token"),
        ("MfaConfirmEnrollRequest", "code"),
        ("MfaLoginCompleteRequest", "code"),
        ("MfaLoginCompleteRequest", "challenge_token"),
    ] {
        assert_schema_property_flag(&spec, schema, property, "writeOnly");
    }
    for (schema, property) in [
        ("MfaChallengeResponse", "challenge_token"),
        ("MfaEnrollResponse", "otpauth_uri"),
        ("MfaEnrollResponse", "secret_base32"),
        ("OAuthStartResponse", "authorize_url"),
        ("OAuthStartResponse", "state"),
    ] {
        assert_schema_property_flag(&spec, schema, property, "readOnly");
    }
}

#[test]
fn openapi_plane_b_authority_has_explicit_direction() {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(
        CreateCredentialRequest,
        UpdateCredentialRequest,
        ResolveCredentialRequest,
        ContinueResolveRequest,
        FormPostField,
        AcquisitionInteraction,
        ResolveCredentialResponse
    )))]
    struct TestDoc;

    let spec = serde_json::to_value(TestDoc::openapi()).expect("schema serializes");
    for (schema, property) in [
        ("CreateCredentialRequest", "data"),
        ("UpdateCredentialRequest", "data"),
        ("ResolveCredentialRequest", "data"),
        ("ContinueResolveRequest", "pending_token"),
        ("ContinueResolveRequest", "user_input"),
    ] {
        assert_schema_property_flag(&spec, schema, property, "writeOnly");
    }
    assert_schema_property_flag(&spec, "FormPostField", "value", "readOnly");

    for property in ["url", "data", "pending_token"] {
        let mut found = Vec::new();
        collect_property_occurrences(&spec, property, &mut found);
        assert!(
            !found.is_empty(),
            "expected generated {property} authority field"
        );
        assert!(
            found
                .iter()
                .any(|value| value.get("readOnly") == Some(&serde_json::Value::Bool(true))),
            "at least one generated {property} field must be readOnly: {found:#?}"
        );
    }
}

#[test]
fn openapi_webhook_authority_has_explicit_direction() {
    use utoipa::OpenApi;

    #[derive(OpenApi)]
    #[openapi(components(schemas(RegisterWebhookRequest, RegisterWebhookResponse)))]
    struct TestDoc;

    let spec = serde_json::to_value(TestDoc::openapi()).expect("schema serializes");
    assert_schema_property_flag(
        &spec,
        "RegisterWebhookRequest",
        "provider_config",
        "writeOnly",
    );
    assert_schema_property_flag(&spec, "RegisterWebhookResponse", "webhook_url", "readOnly");
    assert_schema_property_flag(
        &spec,
        "RegisterWebhookResponse",
        "signing_secret",
        "readOnly",
    );
}
