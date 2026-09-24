//! Existing-id reauthorization through the authenticated public HTTP boundary.
mod common;

use axum::{Router, body::Body, http::StatusCode};
use common::{
    create_state_with_queue, create_test_jwt,
    http_helpers::{auth_get_csrf, auth_json, body_string},
    ws_path,
};
use nebula_api::{
    ApiConfig, app,
    ports::{
        credential_command::test_gateway_from_service,
        credential_service_factory::with_memory_store_parts,
    },
};
use nebula_credential::{
    CredentialContext, CredentialRegistry, DispatchOps, ErasedPendingStore, PendingState,
    SecretString,
    error::CredentialError,
    register_interactive_ops, register_runtime_ops,
    resolve::{InteractionRequest, ResolveResult, StaticResolveResult, UserInput},
    scheme::SecretToken,
};
use nebula_storage::credential::EnvKeyProvider;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn auth_token() -> &'static str {
    static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TOKEN.get_or_init(create_test_jwt)
}

#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct AuthorizationPending {
    secret: String,
}
impl PendingState for AuthorizationPending {
    const KIND: &'static str = "reauthorize_http_pending";
    fn expires_in(&self) -> Duration {
        Duration::from_mins(10)
    }
}
struct InteractiveCredential;
#[nebula_credential::credential(key = "reauthorize_http", name = "HTTP Reauthorization")]
impl InteractiveCredential {
    type Properties = Value;
    type Scheme = SecretToken;
    type State = SecretToken;
    type Pending = AuthorizationPending;
    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }
    async fn resolve(
        _properties: &Value,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        Ok(StaticResolveResult::Complete(SecretToken::new(
            SecretString::new("original"),
        )))
    }
    async fn begin(
        _properties: &Value,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, AuthorizationPending>, CredentialError> {
        Ok(ResolveResult::Pending {
            state: AuthorizationPending {
                secret: "replacement-secret-canary".to_owned(),
            },
            interaction: InteractionRequest::Redirect {
                url: "https://provider.example/authorize".to_owned(),
            },
        })
    }
    async fn continue_resolve(
        pending: &AuthorizationPending,
        _input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, AuthorizationPending>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new(pending.secret.clone()),
        )))
    }
}

async fn router() -> Router {
    let (state, _) = create_state_with_queue().await;
    let mut registry = CredentialRegistry::new();
    registry
        .register(InteractiveCredential, "test")
        .expect("register");
    let mut ops = DispatchOps::new();
    register_runtime_ops::<InteractiveCredential, ErasedPendingStore>(&mut ops)
        .expect("runtime ops");
    register_interactive_ops::<InteractiveCredential, ErasedPendingStore>(&mut ops)
        .expect("interactive ops");
    let key = Arc::new(
        EnvKeyProvider::from_base64("QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=").expect("key"),
    );
    let service = with_memory_store_parts(key, registry, ops)
        .await
        .expect("compose");
    app::build_app(
        state.with_credential_gateway(test_gateway_from_service(service)),
        &ApiConfig::for_test(),
    )
}

async fn send(router: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(auth_json(method, &ws_path(path), auth_token(), &body))
        .await
        .expect("response");
    let status = response.status();
    let raw = body_string(response).await;
    assert!(
        !raw.contains("replacement-secret-canary"),
        "response leaked secret"
    );
    (status, serde_json::from_str(&raw).expect("json"))
}

async fn create(router: &Router, name: &str) -> String {
    let (status, row) = send(
        router,
        "POST",
        "/credentials",
        json!({"credential_key":"reauthorize_http","name":name,"data":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    row["id"].as_str().expect("id").to_owned()
}

async fn begin(router: &Router, id: &str) -> Value {
    let response = router
        .clone()
        .oneshot(auth_json(
            "POST",
            &ws_path(&format!("/credentials/{id}/reauthorize")),
            auth_token(),
            &json!({"data":{}}),
        ))
        .await
        .expect("begin");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let pending: Value = serde_json::from_str(&body_string(response).await).expect("pending");
    assert_eq!(pending["status"], "pending");
    pending
}

async fn complete(router: &Router, pending: &Value) -> (StatusCode, Value) {
    send(router, "POST", "/credentials/resolve/continue", json!({"credential_key":"reauthorize_http", "pending_token":pending["pending_token"], "user_input":"Poll"})).await
}

#[tokio::test]
async fn reauthorization_keeps_existing_id_and_does_not_create_another_row() {
    let router = router().await;
    let id = create(&router, "Existing").await;
    let other = create(&router, "Other").await;
    let pending = begin(&router, &id).await;
    let (status, result) = complete(&router, &pending).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result, json!({"status":"complete","credential_id":id}));
    let response = router
        .clone()
        .oneshot(auth_get_csrf(&ws_path("/credentials"), auth_token()))
        .await
        .expect("list");
    let listed: Value = serde_json::from_str(&body_string(response).await).expect("list json");
    assert_eq!(listed["total"], 2);
    for expected in [&id, &other] {
        assert!(
            listed["credentials"]
                .as_array()
                .expect("rows")
                .iter()
                .any(|row| row["id"] == *expected)
        );
    }
    assert_eq!(
        complete(&router, &pending).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn pending_reauthorization_rejects_intervening_update_and_deletion() {
    for delete in [false, true] {
        let router = router().await;
        let id = create(&router, "Existing").await;
        let pending = begin(&router, &id).await;
        let (status, _) = send(
            &router,
            if delete { "DELETE" } else { "PUT" },
            &format!("/credentials/{id}"),
            if delete {
                json!({})
            } else {
                json!({"data":{}})
            },
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = complete(&router, &pending).await;
        assert_eq!(
            status,
            if delete {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            }
        );
    }
}

#[tokio::test]
async fn continuation_is_session_bound_and_cannot_retarget_or_change_create_intent() {
    let router = router().await;
    let existing = create(&router, "Existing").await;
    let other = create(&router, "Other").await;
    let pending = begin(&router, &existing).await;
    let input = json!({"credential_key":"reauthorize_http", "pending_token":pending["pending_token"], "user_input":"Poll"});
    let stranger = common::me_support::jwt_for(&nebula_core::UserId::new().to_string());
    let response = router
        .clone()
        .oneshot(auth_json(
            "POST",
            &ws_path("/credentials/resolve/continue"),
            &stranger,
            &input,
        ))
        .await
        .expect("stranger response");
    let status = response.status();
    let body = body_string(response).await;
    // A token bound to another authenticated session is rejected as invalid
    // pending data; it remains available to its original session below.
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!body.contains("replacement-secret-canary"));
    let (status, completed) = send(&router, "POST", "/credentials/resolve/continue", json!({"credential_key":"reauthorize_http", "pending_token":pending["pending_token"], "user_input":"Poll", "credential_id":other})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        completed["credential_id"], existing,
        "client field must not retarget the pending intent"
    );

    let (status, new_pending) = send(
        &router,
        "POST",
        "/credentials/resolve",
        json!({"credential_key":"reauthorize_http","data":{}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, created) = send(&router, "POST", "/credentials/resolve/continue", json!({"credential_key":"reauthorize_http", "pending_token":new_pending["pending_token"], "user_input":"Poll", "credential_id":existing})).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        created["credential_id"], existing,
        "a create token cannot replace an existing credential"
    );
    assert_ne!(created["credential_id"], other);
}

#[tokio::test]
async fn reauthorization_requires_authentication_and_hides_missing_targets() {
    let router = router().await;
    let path = ws_path(&format!(
        "/credentials/{}/reauthorize",
        nebula_core::CredentialId::new()
    ));
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(&path)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"data":{}}"#))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = router
        .oneshot(auth_json("POST", &path, auth_token(), &json!({"data":{}})))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[test]
fn reauthorization_request_rejects_caller_supplied_identity_and_fences() {
    use nebula_api::domain::credential::dto::ReauthorizeCredentialRequest;
    for field in [
        "credential_id",
        "credential_key",
        "version",
        "material_epoch",
    ] {
        let mut body = json!({"data":{}});
        body[field] = json!("forged");
        assert!(serde_json::from_value::<ReauthorizeCredentialRequest>(body).is_err());
    }
    let request: ReauthorizeCredentialRequest =
        serde_json::from_value(json!({"data":{"secret":"replacement-secret-canary"}}))
            .expect("request");
    assert!(!format!("{request:?}").contains("replacement-secret-canary"));
}
