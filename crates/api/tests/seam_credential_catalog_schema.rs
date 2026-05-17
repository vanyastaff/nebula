//! ADR-0052 P4 seam (V3 + #6): the credential-type catalog is populated
//! from the `CredentialSchemaPort` (`json_schema()` produced behind the
//! seam); the api-owned public projection strips `x-nebula-root-rules` +
//! predicate operands before the wire. No port ⇒ honest 503.

mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{create_state_with_queue_no_credential_port, create_test_jwt};
use nebula_api::{
    ApiConfig, app,
    ports::credential_schema::{
        CredentialCapabilityFlags, CredentialFieldError, CredentialSchemaPort,
        CredentialTypeDescriptor,
    },
};
use tower::ServiceExt;

const TYPES_PATH: &str = "/api/v1/credentials/types";

/// Yields one credential type whose raw `schema_json` deliberately
/// contains `x-nebula-root-rules` + a predicate-bearing mode operand, to
/// prove the api projection strips them (#6).
struct OneTypePort;

impl CredentialSchemaPort for OneTypePort {
    fn validate_data(
        &self,
        _k: &str,
        _d: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>> {
        Ok(())
    }
    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        vec![self.descriptor()]
    }
    fn get_type(&self, k: &str) -> Option<CredentialTypeDescriptor> {
        (k == "api_key").then(|| self.descriptor())
    }
}

impl OneTypePort {
    fn descriptor(&self) -> CredentialTypeDescriptor {
        CredentialTypeDescriptor {
            key: "api_key".to_owned(),
            name: "API Key".to_owned(),
            description: "Static API key".to_owned(),
            auth_pattern: "SecretToken".to_owned(),
            capabilities: CredentialCapabilityFlags {
                testable: true,
                ..Default::default()
            },
            icon: None,
            documentation_url: None,
            schema_json: serde_json::json!({
                "type": "object",
                "properties": {
                    "api_key": {
                        "type": "string",
                        "minLength": 2,
                        "x-nebula-required-mode": { "when": { "predicate": "secret-cross-field" } }
                    }
                },
                "x-nebula-root-rules": [ { "predicate": "leaked-cross-field-logic" } ],
                "additionalProperties": false
            }),
        }
    }
}

fn auth_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn catalog_lists_types_with_projected_schema() {
    let (state, _q) = common::create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(OneTypePort));
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
    let app = app::build_app(state, &config);

    let resp = app.oneshot(auth_get(TYPES_PATH, &token)).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "catalog must be 200 with a port"
    );
    let text = body_string(resp).await;
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let t0 = &json["types"][0];
    assert_eq!(t0["key"], "api_key");
    assert_eq!(
        t0["schema"]["properties"]["api_key"]["minLength"], 2,
        "standard JSON-Schema keywords are kept (public contract)"
    );
    // #6: cross-field predicate logic MUST NOT leak.
    assert!(
        !text.contains("x-nebula-root-rules"),
        "public schema must strip x-nebula-root-rules; got: {text}"
    );
    assert!(
        !text.contains("leaked-cross-field-logic"),
        "root-rule predicate operands must not leak; got: {text}"
    );
    assert!(
        t0["schema"]["properties"]["api_key"]
            .get("x-nebula-required-mode")
            .is_none(),
        "predicate-bearing mode operand must be stripped; got: {text}"
    );
}

#[tokio::test]
async fn get_credential_type_404_for_unknown_with_port() {
    let (state, _q) = common::create_state_with_queue().await;
    let state = state.with_credential_schema(Arc::new(OneTypePort));
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
    let app = app::build_app(state, &config);

    let resp = app
        .oneshot(auth_get(&format!("{TYPES_PATH}/does-not-exist"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn catalog_503_when_port_unconfigured() {
    let (state, _q) = create_state_with_queue_no_credential_port().await;
    let config = ApiConfig::for_test();
    let token = create_test_jwt();
    let app = app::build_app(state, &config);

    let resp = app.oneshot(auth_get(TYPES_PATH, &token)).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "no port ⇒ honest 503 (unchanged §4.5 stub)"
    );
}
