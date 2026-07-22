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
    ApiConfig, AppState, app, domain::org::InMemoryMembershipStore, state::MembershipStore,
};
use nebula_core::{OrgRole, Principal, UserId};
use tower::ServiceExt;

const CREDENTIAL_ID: &str = "cred_00000000000000000000000001";
const OAUTH_SECRET_CANARY: &str = "oauth-client-secret-NEVER-ECHO-7f3c9a";

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
async fn plane_a_oauth_routes_remain_mounted_and_report_missing_backend_as_503() {
    let (state, _queue) = create_state_with_queue().await;
    let config = ApiConfig::for_test();

    for uri in [
        "/api/v1/auth/oauth/github",
        "/api/v1/auth/oauth/github/callback?state=unused&code=unused",
    ] {
        let response = app::build_app(state.clone(), &config)
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("Plane-A OAuth request"),
            )
            .await
            .expect("Plane-A OAuth response");
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{uri} must remain mounted and auth/CSRF-exempt; an AppState without AuthBackend reports honest 503"
        );
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
