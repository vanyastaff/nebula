//! Permanent boundary regression for the two OAuth planes.
//!
//! Plane A (identity sign-in) remains mounted and intentionally public.
//! Plane B (integration credentials) exposes only the universal
//! `resolve` / `resolve/continue` acquisition protocol; its former raw
//! provider OAuth ceremony is deliberately absent from HTTP.

mod common;

use std::{path::Path, sync::Arc};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN, TEST_ORG, create_state_with_queue, me_support::jwt_for,
    ws_path,
};
use nebula_api::{
    ApiConfig, AppState, app,
    domain::{auth::backend::InMemoryAuthBackend, org::InMemoryMembershipStore},
    state::MembershipStore,
};
use nebula_core::{OrgRole, Principal, UserId};
use tower::ServiceExt;

const CREDENTIAL_ID: &str = "cred_00000000000000000000000001";
const OAUTH_SECRET_CANARY: &str = "oauth-client-secret-NEVER-ECHO-7f3c9a";
const VALID_OAUTH_STATE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

async fn state_with_real_credential_rbac() -> (AppState, String) {
    let (state, _queue) = create_state_with_queue().await;
    let user_id = UserId::new();
    let membership = InMemoryMembershipStore::seeded(
        TEST_ORG.parse().expect("valid test org id"),
        Principal::User(user_id),
        OrgRole::OrgAdmin,
    )
    .into_arc();
    let membership: Arc<dyn MembershipStore> = membership;

    (
        state.with_membership_store(membership),
        jwt_for(&user_id.to_string()),
    )
}

fn protected_request(method: Method, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("code=unused&state=unused"))
        .expect("protected OAuth regression request")
}

fn protected_json_request(uri: &str, token: &str, body: &serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(body).expect("serialize credential resolve request"),
        ))
        .expect("protected credential resolve request")
}

async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    String::from_utf8(bytes.to_vec()).expect("API response body is UTF-8")
}

#[tokio::test]
async fn former_credential_oauth_operations_are_exact_404_after_real_auth_and_rbac() {
    let (state, token) = state_with_real_credential_rbac().await;
    let config = ApiConfig::for_test();

    let control = app::build_app(state.clone(), &config)
        .oneshot(protected_request(
            Method::GET,
            &ws_path("/credentials"),
            &token,
        ))
        .await
        .expect("credential control response");
    assert_eq!(
        control.status(),
        StatusCode::OK,
        "the same bearer, membership, tenant resolution, grant, and RBAC context must reach a protected credential handler"
    );

    let system_auth = format!(
        "/api/v1/credentials/{CREDENTIAL_ID}/oauth2/auth?auth_url=https%3A%2F%2Fprovider.example%2Fauthorize&client_id=client&redirect_uri=https%3A%2F%2Fapp.example%2Fcallback"
    );
    let system_callback =
        format!("/api/v1/credentials/{CREDENTIAL_ID}/oauth2/callback?code=unused&state=unused");
    let scoped_auth = ws_path(&format!(
        "/credentials/{CREDENTIAL_ID}/oauth2/auth?auth_url=https%3A%2F%2Fprovider.example%2Fauthorize&client_id=client&redirect_uri=https%3A%2F%2Fapp.example%2Fcallback"
    ));
    let scoped_callback = ws_path(&format!(
        "/credentials/{CREDENTIAL_ID}/oauth2/callback?code=unused&state=unused"
    ));

    for (label, method, uri) in [
        ("system authorize GET", Method::GET, system_auth.as_str()),
        ("system callback GET", Method::GET, system_callback.as_str()),
        (
            "system callback POST",
            Method::POST,
            system_callback.as_str(),
        ),
        ("scoped authorize GET", Method::GET, scoped_auth.as_str()),
        ("scoped callback GET", Method::GET, scoped_callback.as_str()),
        (
            "scoped callback POST",
            Method::POST,
            scoped_callback.as_str(),
        ),
    ] {
        let response = app::build_app(state.clone(), &config)
            .oneshot(protected_request(method, uri, &token))
            .await
            .unwrap_or_else(|error| panic!("{label} response failed: {error}"));
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap_or_else(|error| panic!("{label} body failed: {error}"));
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{label} must be absent, not rejected by parsing/auth/RBAC/CSRF or retained as a tombstone; body={}",
            String::from_utf8_lossy(&body)
        );
    }
}

#[tokio::test]
async fn plane_a_oauth_routes_remain_mounted_and_fail_closed_without_composition_or_binding() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();
    let state = state.with_public_url(config.public_url.clone());

    for (uri, expected) in [
        (
            "/api/v1/auth/oauth/github".to_owned(),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (
            format!("/api/v1/auth/oauth/github/callback?state={VALID_OAUTH_STATE}&code=unused"),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let response = app::build_app(state.clone(), &config)
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(&uri)
                    .header("host", "127.0.0.1:0")
                    .body(Body::empty())
                    .expect("Plane-A OAuth request"),
            )
            .await
            .expect("Plane-A OAuth response");
        assert_eq!(
            response.status(),
            expected,
            "{uri} must remain mounted and auth/CSRF-exempt; start reports missing composition as 503 while a callback without its browser binding fails before backend dispatch as 401"
        );
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store"),
            "{uri} must never be cached, including problem responses"
        );
        assert_eq!(
            response
                .headers()
                .get("pragma")
                .and_then(|value| value.to_str().ok()),
            Some("no-cache"),
            "{uri} must carry the legacy no-cache directive"
        );
        assert_eq!(
            response
                .headers()
                .get("referrer-policy")
                .and_then(|value| value.to_str().ok()),
            Some("no-referrer"),
            "{uri} must not refer callback material to another origin"
        );
    }
}

#[tokio::test]
async fn malformed_identity_oauth_callback_is_rejected_before_backend_dispatch() {
    let config = ApiConfig::for_test();
    let state = AppState::in_memory(config.jwt_secret.clone())
        .with_auth_backend(InMemoryAuthBackend::new().into_arc())
        .with_public_url(config.public_url.clone());

    for (label, query) in [
        ("short state", "state=too-short&code=visible-code"),
        (
            "empty code",
            concat!(
                "state=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "&code="
            ),
        ),
    ] {
        let response = app::build_app(state.clone(), &config)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/auth/oauth/github/callback?{query}"))
                    .body(Body::empty())
                    .expect("malformed Plane-A OAuth callback request"),
            )
            .await
            .unwrap_or_else(|error| panic!("{label} callback response failed: {error}"));

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{label} must fail at the transport boundary; this backend would otherwise report OAuth disabled as 503"
        );
    }
}

#[tokio::test]
async fn oauth_callback_query_rejections_are_fixed_problem_details_without_cookie_clear() {
    const QUERY_CANARY: &str = "DUPLICATE_QUERY_CANARY_DO_NOT_ECHO";
    let config = ApiConfig::for_test();
    let state = AppState::in_memory(config.jwt_secret.clone())
        .with_auth_backend(InMemoryAuthBackend::new().into_arc())
        .with_public_url(config.public_url.clone());

    for (label, query) in [
        (
            "duplicate field",
            format!("state={VALID_OAUTH_STATE}&state={QUERY_CANARY}&code=visible-code"),
        ),
        (
            "invalid UTF-8 escape",
            "state=%FF&code=visible-code".to_owned(),
        ),
    ] {
        let response = app::build_app(state.clone(), &config)
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/auth/oauth/github/callback?{query}"))
                    .body(Body::empty())
                    .expect("malformed OAuth query request"),
            )
            .await
            .unwrap_or_else(|error| panic!("{label} response failed: {error}"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/problem+json")
        );
        assert!(
            response
                .headers()
                .get_all("set-cookie")
                .iter()
                .next()
                .is_none(),
            "query rejection happens before browser binding and must not clear a transaction cookie"
        );
        let body = response_body(response).await;
        assert!(!body.contains(QUERY_CANARY));
        assert!(!body.contains("duplicate field"));
        assert!(!body.contains("invalid utf-8"));
    }
}

#[tokio::test]
async fn default_credential_composition_parks_oauth2_until_universal_pending_flow_exists() {
    let (state, token) = state_with_real_credential_rbac().await;
    let config = ApiConfig::for_test();

    let catalog_response = app::build_app(state.clone(), &config)
        .oneshot(protected_request(
            Method::GET,
            "/api/v1/credentials/types",
            &token,
        ))
        .await
        .expect("credential catalog response");
    assert_eq!(catalog_response.status(), StatusCode::OK);
    let catalog: serde_json::Value =
        serde_json::from_str(&response_body(catalog_response).await).expect("credential catalog");
    let advertised_keys: Vec<&str> = catalog["types"]
        .as_array()
        .expect("catalog types array")
        .iter()
        .filter_map(|credential_type| credential_type["key"].as_str())
        .collect();
    assert!(
        advertised_keys.contains(&"api_key"),
        "a supported static type must remain advertised as a positive control: {advertised_keys:?}"
    );

    let oauth_type_response = app::build_app(state.clone(), &config)
        .oneshot(protected_request(
            Method::GET,
            "/api/v1/credentials/types/oauth2",
            &token,
        ))
        .await
        .expect("parked OAuth2 type response");
    let oauth_type_status = oauth_type_response.status();

    let resolve_response = app::build_app(state, &config)
        .oneshot(protected_json_request(
            &ws_path("/credentials/resolve"),
            &token,
            &serde_json::json!({
                "credential_key": "oauth2",
                "data": {
                    "client_id": "parked-client",
                    "client_secret": OAUTH_SECRET_CANARY,
                    "token_url": "https://provider.example.com/oauth/token",
                    "grant_type": "client_credentials"
                }
            }),
        ))
        .await
        .expect("parked OAuth2 resolve response");
    let resolve_status = resolve_response.status();
    let resolve_body = response_body(resolve_response).await;
    assert!(
        !resolve_body.contains(OAUTH_SECRET_CANARY),
        "unknown-type rejection must never echo caller-supplied secret material: {resolve_body}"
    );

    assert_eq!(
        (
            advertised_keys.contains(&"oauth2"),
            oauth_type_status,
            resolve_status,
        ),
        (false, StatusCode::NOT_FOUND, StatusCode::BAD_REQUEST),
        "the default public composition must neither advertise nor dispatch the parked oauth2 type"
    );
    let resolve_problem: serde_json::Value =
        serde_json::from_str(&resolve_body).expect("unknown-type ProblemDetails");
    assert_eq!(
        resolve_problem["errors"][0]["code"], "unknown_credential_type",
        "resolve must fail with the structured unknown-type classification: {resolve_body}"
    );
    assert_eq!(
        resolve_problem["errors"][0]["pointer"], "/credential_key",
        "the unknown-type error must identify the credential-key field: {resolve_body}"
    );
}

#[test]
fn removed_credential_oauth_source_surface_cannot_regrow_accidentally() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for removed_file in [
        "src/domain/credential/oauth.rs",
        "src/transport/oauth/state.rs",
    ] {
        assert!(
            !root.join(removed_file).exists(),
            "removed Plane-B ceremony source file must stay absent: {removed_file}"
        );
    }

    let source_files = [
        "src/domain/credential/mod.rs",
        "src/domain/credential/routes.rs",
        "src/domain/credential/handler.rs",
        "src/domain/workspace.rs",
        "src/state.rs",
        "src/transport/credential.rs",
        "src/transport/oauth/mod.rs",
    ];
    let source = source_files
        .iter()
        .map(|path| {
            std::fs::read_to_string(root.join(path))
                .unwrap_or_else(|error| panic!("read {path}: {error}"))
        })
        .collect::<Vec<_>>()
        .join("\n");

    for removed_name in [
        "get_oauth2_authorize_url",
        "get_oauth2_callback",
        "post_oauth2_callback",
        "get_oauth2_authorize_url_for_owner",
        "handle_callback_for_owner",
        "oauth_controller",
        "oauth_pending_store",
        "oauth_state_tokens",
        "RequestCredentialOwner",
        "owner_id_from_scope",
        "scoped_store",
        "AuthUriResponse",
        "CallbackResponse",
    ] {
        assert!(
            !source.contains(removed_name),
            "removed Plane-B route/controller/wrapper symbol must stay absent: {removed_name}"
        );
    }
}
