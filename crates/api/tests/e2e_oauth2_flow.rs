#![cfg(feature = "credential-oauth")]

mod common;

use axum::{
    Json,
    body::Body,
    extract::Form,
    http::{Request, StatusCode},
    routing::post,
};
use common::{create_state_with_queue, create_test_jwt};
use nebula_api::{ApiConfig, app};
use nebula_credential::{
    Credential, CredentialContext, CredentialState, CredentialStore, OAuth2Credential, OAuth2State,
    PutMode,
};
use nebula_engine::credential::CredentialResolver;
use tower::ServiceExt;
use url::form_urlencoded;

async fn spawn_mock_token_endpoint() -> (String, tokio::task::JoinHandle<()>) {
    let token_router = axum::Router::new().route(
        "/token",
        post(
            |Form(form): Form<std::collections::HashMap<String, String>>| async move {
                let grant_type = form
                    .get("grant_type")
                    .map(String::as_str)
                    .unwrap_or_default();
                let body = match grant_type {
                    "authorization_code" => serde_json::json!({
                        "access_token": "e2e-access-token",
                        "refresh_token": "e2e-refresh-token",
                        "token_type": "Bearer",
                        "expires_in": 1800,
                        "scope": "repo workflow"
                    }),
                    "refresh_token" => serde_json::json!({
                        "access_token": "e2e-access-token-refreshed",
                        "refresh_token": "e2e-refresh-token-refreshed",
                        "token_type": "Bearer",
                        "expires_in": 3600,
                        "scope": "repo workflow"
                    }),
                    _ => serde_json::json!({
                        "error": "unsupported_grant_type",
                        "error_description": "unknown grant_type"
                    }),
                };
                Json(body)
            },
        ),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock token endpoint");
    let addr = listener.local_addr().expect("mock token endpoint addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, token_router).await;
    });

    (format!("http://{addr}/token"), handle)
}

#[tokio::test]
async fn e2e_oauth2_flow_persists_exchanged_credential_state() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state.clone(), &config);
    let token = create_test_jwt();
    let credential_id = "oauth-e2e-credential";
    let client_id = "e2e-client-id";
    let client_secret = "e2e-client-secret";
    let redirect_uri = "https://app.example.com/oauth/callback";
    let auth_url = "https://provider.example.com/oauth/authorize";
    let (token_url, token_server_handle) = spawn_mock_token_endpoint().await;

    let auth_query = form_urlencoded::Serializer::new(String::new())
        .append_pair("auth_url", auth_url)
        .append_pair("token_url", &token_url)
        .append_pair("client_id", client_id)
        .append_pair("client_secret", client_secret)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scopes", "repo workflow")
        .finish();
    let auth_uri = format!("/api/v1/credentials/{credential_id}/oauth2/auth?{auth_query}");

    let auth_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(auth_uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("oauth auth request"),
        )
        .await
        .expect("oauth auth response");
    assert_eq!(auth_response.status(), StatusCode::OK);

    let auth_body = axum::body::to_bytes(auth_response.into_body(), usize::MAX)
        .await
        .expect("oauth auth body");
    let auth_json: serde_json::Value =
        serde_json::from_slice(&auth_body).expect("oauth auth response json");
    let signed_state = auth_json["state"]
        .as_str()
        .expect("signed state")
        .to_owned();
    assert!(
        auth_json["authorize_url"]
            .as_str()
            .is_some_and(|url| url.starts_with(auth_url)),
        "authorize_url should be built from provider auth_url"
    );

    let callback_query = form_urlencoded::Serializer::new(String::new())
        .append_pair("code", "e2e-auth-code")
        .append_pair("state", &signed_state)
        .finish();
    let callback_uri =
        format!("/api/v1/credentials/{credential_id}/oauth2/callback?{callback_query}");

    let callback_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(callback_uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("oauth callback request"),
        )
        .await
        .expect("oauth callback response");
    assert_eq!(callback_response.status(), StatusCode::OK);

    let callback_body = axum::body::to_bytes(callback_response.into_body(), usize::MAX)
        .await
        .expect("oauth callback body");
    let callback_json: serde_json::Value =
        serde_json::from_slice(&callback_body).expect("oauth callback response json");
    assert_eq!(callback_json["credential_id"], credential_id);
    assert_eq!(callback_json["exchanged"], true);

    let stored = state
        .oauth_credential_store
        .get(credential_id)
        .await
        .expect("stored oauth credential");
    assert_eq!(stored.credential_key, OAuth2Credential::KEY);
    assert_eq!(stored.state_kind, OAuth2State::KIND);

    let mut persisted_state: OAuth2State =
        serde_json::from_slice(&stored.data).expect("persisted oauth state");
    assert_eq!(
        persisted_state.access_token.expose_secret(|s| s.to_owned()),
        "e2e-access-token"
    );
    assert_eq!(
        persisted_state
            .refresh_token
            .as_ref()
            .expect("refresh token")
            .expose_secret(|s| s.to_owned()),
        "e2e-refresh-token"
    );
    assert_eq!(persisted_state.scopes, vec!["repo", "workflow"]);
    assert_eq!(
        persisted_state.client_id.expose_secret(|s| s.to_owned()),
        client_id
    );
    assert_eq!(persisted_state.token_url, token_url);

    // Force state into early-refresh window to exercise engine refresh path.
    persisted_state.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(30));
    let mut stale_record = stored.clone();
    stale_record.data = serde_json::to_vec(&persisted_state).expect("serialize stale oauth state");
    stale_record.expires_at = persisted_state.expires_at();
    state
        .oauth_credential_store
        .put(stale_record, PutMode::Overwrite)
        .await
        .expect("persist stale oauth state");

    let resolver = CredentialResolver::new(state.oauth_credential_store.clone());
    let ctx = CredentialContext::new("test-user");
    resolver
        .resolve_with_refresh::<OAuth2Credential>(credential_id, &ctx)
        .await
        .expect("resolve with refresh should succeed");

    let refreshed = state
        .oauth_credential_store
        .get(credential_id)
        .await
        .expect("refreshed oauth credential in store");
    let refreshed_state: OAuth2State =
        serde_json::from_slice(&refreshed.data).expect("deserialize refreshed oauth state");
    assert_eq!(
        refreshed_state.access_token.expose_secret(|s| s.to_owned()),
        "e2e-access-token-refreshed"
    );
    assert_eq!(
        refreshed_state
            .refresh_token
            .expect("refreshed refresh token")
            .expose_secret(|s| s.to_owned()),
        "e2e-refresh-token-refreshed"
    );

    token_server_handle.abort();
}
