// `expose_secret` needs a higher-ranked closure; `|s| s.to_owned()` is required (clippy false
// positive).
#![allow(clippy::redundant_closure_for_method_calls)]

mod common;

use axum::{
    Json,
    body::Body,
    extract::Form,
    http::{Request, StatusCode},
    routing::post,
};
use common::{create_state_with_queue, create_test_jwt, ws_path};
use nebula_api::ports::ReqwestRefreshTransport;
use nebula_api::{ApiConfig, app};
use nebula_credential::{
    Credential, CredentialContext, CredentialState, CredentialStore, ErasedCredentialStore,
    OAuth2Credential, OAuth2State, PutMode,
};
use nebula_engine::credential::{CredentialResolver, default_in_memory_coordinator};
use tower::ServiceExt;
use url::form_urlencoded;

/// The single credential store: the facade's layered store handle (the
/// OAuth path and the CRUD plane share it — ADR-0088 D7).
fn oauth_store_handle(state: &nebula_api::AppState) -> ErasedCredentialStore {
    ErasedCredentialStore::new(
        state
            .credential_service
            .as_ref()
            .expect("credential service wired into test state")
            .credential_store_handle(),
    )
}

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
async fn system_level_oauth_authorize_route_is_disabled() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state, &config);
    let token = create_test_jwt();

    let auth_query = form_urlencoded::Serializer::new(String::new())
        .append_pair("auth_url", "https://provider.example.com/oauth/authorize")
        .append_pair("token_url", "https://provider.example.com/oauth/token")
        .append_pair("client_id", "client")
        .append_pair("client_secret", "secret")
        .append_pair("redirect_uri", "https://app.example.com/oauth/callback")
        .finish();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/credentials/cred_00000000000000000000000001/oauth2/auth?{auth_query}"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("oauth auth request"),
        )
        .await
        .expect("oauth auth response");

    assert_eq!(
        response.status(),
        StatusCode::GONE,
        "OAuth credential flow must be tenant-scoped, not system-level"
    );
}

#[tokio::test]
async fn oauth_authorize_rejects_loopback_token_url() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state, &config);
    let token = create_test_jwt();

    let auth_query = form_urlencoded::Serializer::new(String::new())
        .append_pair("auth_url", "https://provider.example.com/oauth/authorize")
        .append_pair("token_url", "http://127.0.0.1:1/token")
        .append_pair("client_id", "client")
        .append_pair("client_secret", "secret")
        .append_pair("redirect_uri", "https://app.example.com/oauth/callback")
        .finish();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(ws_path(&format!(
                    "/credentials/cred_00000000000000000000000001/oauth2/auth?{auth_query}"
                )))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("oauth auth request"),
        )
        .await
        .expect("oauth auth response");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "OAuth token endpoint must reject loopback/private URLs before exchange"
    );
}

// IGNORED 2026-04-24: test exercises engine's OAuth2 refresh path which requires
// the `rotation` feature on nebula-engine. That feature currently fails to compile
// due to missing `validation` submodule in nebula-credential::rotation (pre-existing
// gap — `validation` is referenced by engine/src/credential/rotation.rs:34 but never
// existed in credential crate after earlier refactor). Previously this test was
// gated behind `credential-oauth` feature which effectively never ran in CI.
// Un-ignore when engine's rotation feature compiles end-to-end (separate spec).
#[ignore = "requires engine rotation feature which currently has broken module graph"]
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

    let stored = oauth_store_handle(&state)
        .get(credential_id)
        .await
        .expect("stored oauth credential");
    assert_eq!(
        stored.version, 2,
        "authorize creates the placeholder (v1); the callback CAS bumps it to v2"
    );
    assert_eq!(stored.credential_key, OAuth2Credential::KEY);
    assert_eq!(stored.state_kind, OAuth2State::KIND);

    let mut persisted_state: OAuth2State =
        serde_json::from_slice(&stored.data).expect("persisted oauth state");
    assert_eq!(
        persisted_state.access_token.expose_secret().to_owned(),
        "e2e-access-token"
    );
    assert_eq!(
        persisted_state
            .refresh_token
            .as_ref()
            .expect("refresh token")
            .expose_secret()
            .to_owned(),
        "e2e-refresh-token"
    );
    assert_eq!(persisted_state.scopes, vec!["repo", "workflow"]);
    assert_eq!(
        persisted_state.client_id.expose_secret().to_owned(),
        client_id
    );
    assert_eq!(persisted_state.token_url, token_url);

    // Force state into early-refresh window to exercise engine refresh path.
    persisted_state.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(30));
    let mut stale_record = stored.clone();
    stale_record.data = serde_json::to_vec(&persisted_state).expect("serialize stale oauth state");
    stale_record.expires_at = persisted_state.expires_at();
    let stale_put = oauth_store_handle(&state)
        .put(stale_record, PutMode::Overwrite)
        .await
        .expect("persist stale oauth state");
    assert_eq!(
        stale_put.version, 3,
        "manual overwrite of stale state should bump StoredCredential::version (CAS basis)"
    );

    let coord = std::sync::Arc::new(
        default_in_memory_coordinator()
            .expect("default in-memory coordinator must build with static config"),
    );
    let transport = std::sync::Arc::new(ReqwestRefreshTransport);
    let resolver = CredentialResolver::with_dependencies(
        std::sync::Arc::new(oauth_store_handle(&state)),
        coord,
        transport,
    );
    let ctx = CredentialContext::for_test("test-user");
    let handle = resolver
        .resolve_with_refresh::<OAuth2Credential>(credential_id, &ctx)
        .await
        .expect("resolve with refresh should succeed");
    assert_eq!(handle.credential_id(), credential_id);
    let token = handle.snapshot();
    assert_eq!(token.token_type, "Bearer");
    assert_eq!(token.scopes, vec!["repo".to_owned(), "workflow".to_owned()]);
    assert_eq!(
        token.access_token().expose_secret().to_owned(),
        "e2e-access-token-refreshed"
    );

    let refreshed = oauth_store_handle(&state)
        .get(credential_id)
        .await
        .expect("refreshed oauth credential in store");
    assert_eq!(
        refreshed.version, 4,
        "engine refresh should persist via CAS and increment version"
    );
    let refreshed_state: OAuth2State =
        serde_json::from_slice(&refreshed.data).expect("deserialize refreshed oauth state");
    assert_eq!(
        refreshed_state.access_token.expose_secret().to_owned(),
        "e2e-access-token-refreshed"
    );
    assert_eq!(
        refreshed_state
            .refresh_token
            .clone()
            .expect("refreshed refresh token")
            .expose_secret()
            .to_owned(),
        "e2e-refresh-token-refreshed"
    );

    token_server_handle.abort();
}

#[tokio::test]
async fn system_level_oauth_callback_post_route_is_disabled() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state, &config);
    let token = create_test_jwt();

    let callback_body = form_urlencoded::Serializer::new(String::new())
        .append_pair("code", "unused-auth-code")
        .append_pair("state", "unused-signed-state")
        .finish();
    let credential_id = "cred_00000000000000000000000001";
    let callback_uri = format!("/api/v1/credentials/{credential_id}/oauth2/callback");

    // Provide the double-submit CSRF pair: `csrf_middleware` now runs on
    // `credential_routes` (M3.1 box 2). Without the headers the request
    // would be rejected with 403 *before* reaching the disabled route,
    // hiding the 410-GONE contract this test is asserting.
    let callback_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(callback_uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/x-www-form-urlencoded")
                .header("x-csrf-token", common::TEST_CSRF_TOKEN)
                .header("cookie", common::TEST_CSRF_COOKIE)
                .body(Body::from(callback_body))
                .expect("oauth callback POST"),
        )
        .await
        .expect("oauth callback response");
    assert_eq!(
        callback_response.status(),
        StatusCode::GONE,
        "OAuth form_post callback must also be tenant-scoped"
    );
}

/// Direct coverage of the newly-CSRF-gated system-level
/// `credential::routes::router()` surface (M3.1 box 2).
///
/// `seam_credential_write_path_validation` exercises the tenant-scoped
/// `/orgs/.../workspaces/.../credentials/*` group, which had CSRF
/// enforcement applied long before this PR. The system-level
/// `/api/v1/credentials/*` group only got `csrf_middleware` in this PR,
/// so verify the contract directly: a state-changing POST against
/// `POST /api/v1/credentials/{id}/oauth2/callback` without the
/// double-submit pair must be rejected at 403 by `csrf_middleware`
/// *before* reaching the disabled-route 410 handler.
#[tokio::test]
async fn system_level_oauth_callback_post_requires_csrf_pair() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let app = app::build_app(state, &config);
    let token = create_test_jwt();

    let callback_body = form_urlencoded::Serializer::new(String::new())
        .append_pair("code", "unused-auth-code")
        .append_pair("state", "unused-signed-state")
        .finish();
    let credential_id = "cred_00000000000000000000000001";
    let callback_uri = format!("/api/v1/credentials/{credential_id}/oauth2/callback");

    // No `x-csrf-token` header, no `cookie` header — JWT auth alone.
    // The expected response is the CSRF 403, NOT the disabled-route 410,
    // proving that `csrf_middleware` runs before the route handler.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(callback_uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(callback_body))
                .expect("oauth callback POST without CSRF"),
        )
        .await
        .expect("oauth callback response");
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "csrf_middleware must reject the system-level OAuth POST when the \
         double-submit CSRF pair is absent, even though the route itself \
         is also disabled (410)"
    );
}
