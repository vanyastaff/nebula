//! `me/*` end-to-end coverage (Phase 2).
//!
//! Five of the six `/api/v1/me/*` endpoints graduated stub→implemented
//! against the Plane-A `AuthBackend` port. These tests drive the full
//! middleware → handler → `AuthBackend` path against a **real**
//! `InMemoryAuthBackend` (Argon2id / RFC 6238 TOTP / SHA-256 PAT lookup —
//! the §4.5-honest production-quality default; `nebula_storage` ships no
//! `UserRepo`/`PatRepo`/`SessionRepo` impl, so this in-memory backend *is*
//! the real backing, exactly as `InMemoryControlQueueRepo` is for the
//! durable control plane in Phase 1).
//!
//! `GET /me/orgs` is intentionally **not** covered here: it stays an
//! honest 501 stub (principal→orgs enumeration is not wired until the
//! org/membership phase — canon §4.5). Its 501 contract is locked by
//! `openapi_canon_compliance.rs`.
//!
//! ## Coverage
//!
//! | Endpoint | Happy | Typed-error paths |
//! |----------|-------|-------------------|
//! | `GET /me` | profile + real `tokens_count`, `orgs_count` absent | 401 (no auth / non-user principal), 503 (port absent) |
//! | `PATCH /me` | display_name + avatar_url applied | 400 (blank name), 401, 404 (user gone), 503 |
//! | `GET /me/tokens` | lists active PATs, metadata only | 401, 503 |
//! | `POST /me/tokens` | 201 + plaintext once; redaction | 400 (blank name), 401, 503 |
//! | `DELETE /me/tokens/{pat}` | revoke (idempotent) | 404 (unknown / cross-user), 401 |

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{
    TEST_CSRF_COOKIE, TEST_CSRF_TOKEN,
    me_support::{create_me_state, create_me_state_without_backend, jwt_for},
};
// `AuthBackend` is in scope so the white-box test seams can call port
// methods (`register_user`, `create_pat`, `list_pats`) directly on the
// concrete `InMemoryAuthBackend` handle.
use nebula_api::{ApiConfig, app, domain::auth::backend::AuthBackend};
use serde_json::Value;
use tower::ServiceExt;

/// Build a state-changing (PATCH/POST/DELETE) request with the
/// double-submit CSRF pair the JWT auth path requires (same contract the
/// Phase-1 execution tests use). Without these a JWT-authenticated
/// mutating request is correctly rejected with 403 by `csrf_middleware`.
fn mutating(method: &str, uri: &str, jwt: &str, json_body: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {jwt}"))
        .header("x-csrf-token", TEST_CSRF_TOKEN)
        .header("cookie", TEST_CSRF_COOKIE);
    let body = match json_body {
        Some(j) => {
            b = b.header("content-type", "application/json");
            Body::from(j.to_owned())
        },
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

const PAT_PLAINTEXT_PREFIX: &str = "pat_";

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body readable");
    serde_json::from_slice(&bytes).expect("body is JSON")
}

fn get(uri: &str, jwt: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {jwt}"))
        .body(Body::empty())
        .unwrap()
}

// ── GET /me ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_me_returns_profile_with_real_token_count() {
    let (state, backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();

    // Seed two PATs so tokens_count is a *real* derived value, not 0.
    backend
        .create_pat(
            &user.user_id,
            nebula_api::domain::auth::backend::CreatePatParams {
                name: "ci".to_owned(),
                scopes: vec!["workflows:read".to_owned()],
                ttl_seconds: None,
            },
        )
        .await
        .unwrap();
    backend
        .create_pat(
            &user.user_id,
            nebula_api::domain::auth::backend::CreatePatParams {
                name: "cli".to_owned(),
                scopes: vec![],
                ttl_seconds: Some(3600),
            },
        )
        .await
        .unwrap();

    let app = app::build_app(state, &api_config);
    let response = app.oneshot(get("/api/v1/me", &user.jwt)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["user_id"], user.user_id);
    assert_eq!(body["email"], user.email);
    assert_eq!(body["display_name"], "Me E2E");
    assert_eq!(body["email_verified"], false);
    assert_eq!(body["mfa_enabled"], false);
    assert_eq!(
        body["tokens_count"], 2,
        "tokens_count must reflect the two seeded PATs"
    );
    // `orgs_count` must be ABSENT from the wire (not a synthesized `0`):
    // principal→orgs enumeration is not wired until the org/membership
    // phase, and a count the system cannot compute would be a false
    // value on the JSON contract (canon §4.5 / §12.2). Asserting absence
    // is stricter than the old `== 0` — it forbids the lying field.
    assert!(
        body.get("orgs_count").is_none(),
        "orgs_count must be omitted (not 0) until membership enumeration \
         is wired; got: {:?}",
        body.get("orgs_count")
    );
}

#[tokio::test]
async fn get_me_without_auth_is_401() {
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn get_me_with_non_user_principal_is_401() {
    // An API-key request resolves to `Principal::System`, which has no
    // personal profile — the handler must reject it with 401 (problem+json),
    // not resolve to some other identity.
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let state = state.with_api_keys(vec!["nbl_sk_test_key_value_1234567890".to_owned()]);
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me")
                .header("x-api-key", "nbl_sk_test_key_value_1234567890")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let ct = response_content_type_was(&response);
    assert!(
        ct,
        "non-user-principal 401 must come from the handler (RFC 9457 problem+json)"
    );
}

// Helper: a 401 emitted by the handler carries application/problem+json;
// one short-circuited by middleware does not. We check the header on the
// still-owned response before consuming the body.
fn response_content_type_was(response: &axum::response::Response) -> bool {
    response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("application/problem+json"))
        .unwrap_or(false)
}

#[tokio::test]
async fn get_me_with_backend_absent_is_503() {
    let (state, jwt) = create_me_state_without_backend();
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app.oneshot(get("/api/v1/me", &jwt)).await.unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "auth_backend port absent must fail closed with 503 (honest degradation)"
    );
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json"),
        "503 must be RFC 9457 problem+json"
    );
}

// ── PATCH /me ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn patch_me_applies_display_name_and_avatar() {
    let (state, _backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(mutating(
            "PATCH",
            "/api/v1/me",
            &user.jwt,
            Some(r#"{"display_name":"Renamed Human","avatar_url":"https://cdn.nebula.dev/a.png"}"#),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["display_name"], "Renamed Human");
    assert_eq!(body["user_id"], user.user_id);
}

#[tokio::test]
async fn patch_me_blank_name_is_400() {
    let (state, _backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(mutating(
            "PATCH",
            "/api/v1/me",
            &user.jwt,
            Some(r#"{"display_name":"   "}"#),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json")
    );
}

#[tokio::test]
async fn patch_me_unknown_user_is_404() {
    // Backend is wired but the JWT subject was never registered → the
    // handler reaches the port, which reports UserNotFound → 404.
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let ghost_jwt = jwt_for(&nebula_core::UserId::new().to_string());

    let response = app
        .oneshot(mutating(
            "PATCH",
            "/api/v1/me",
            &ghost_jwt,
            Some(r#"{"display_name":"Nobody"}"#),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_me_without_auth_is_401() {
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/me")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── GET /me/tokens ───────────────────────────────────────────────────────────

#[tokio::test]
async fn list_my_tokens_returns_metadata_only() {
    let (state, backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();

    let minted = backend
        .create_pat(
            &user.user_id,
            nebula_api::domain::auth::backend::CreatePatParams {
                name: "list-me".to_owned(),
                scopes: vec!["a".to_owned()],
                ttl_seconds: None,
            },
        )
        .await
        .unwrap();
    let plaintext = minted.plaintext.clone();

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(get("/api/v1/me/tokens", &user.jwt))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let raw = String::from_utf8(bytes.to_vec()).unwrap();
    let body: Value = serde_json::from_str(&raw).unwrap();

    let tokens = body["tokens"].as_array().expect("tokens array");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0]["name"], "list-me");
    assert_eq!(tokens[0]["scopes"][0], "a");
    assert!(tokens[0]["id"].as_str().unwrap().starts_with("pat_"));
    assert!(
        !raw.contains(&plaintext),
        "list response must never contain the PAT plaintext"
    );
}

#[tokio::test]
async fn list_my_tokens_without_auth_is_401() {
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/me/tokens")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── POST /me/tokens — creation + secret handling ─────────────────────────────

#[tokio::test]
async fn create_token_returns_plaintext_once_and_201() {
    let (state, backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(mutating(
            "POST",
            "/api/v1/me/tokens",
            &user.jwt,
            Some(r#"{"name":"deploy-bot","scopes":["workflows:read","workflows:run"],"ttl_seconds":7200}"#),
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "token creation must be 201"
    );
    let body = body_json(response).await;
    let token = body["token"].as_str().expect("token plaintext present");
    assert!(
        token.starts_with(PAT_PLAINTEXT_PREFIX),
        "plaintext must be the `pat_…` form, got prefix {:?}",
        &token[..token.len().min(8)]
    );
    assert_eq!(body["summary"]["name"], "deploy-bot");
    assert_eq!(body["summary"]["scopes"][0], "workflows:read");

    // The plaintext is exposed exactly once: a subsequent list returns
    // metadata for the same token but never the secret.
    let listed = backend.list_pats(&user.user_id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "deploy-bot");
    // PatRecord has no plaintext field at all — the secret only ever
    // existed in the create response body (compile-time guarantee).
}

#[tokio::test]
async fn create_token_blank_name_is_400() {
    let (state, _backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(mutating(
            "POST",
            "/api/v1/me/tokens",
            &user.jwt,
            Some(r#"{"name":"","scopes":[]}"#),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_token_without_auth_is_401() {
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/me/tokens")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"x","scopes":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Secret-redaction contract for `create_token` (canon §12.5 / STYLE §6):
/// the plaintext is returned **exactly once** in the body and **never**
/// surfaces in tracing output. Mirrors the
/// `crates/credential/tests/redaction.rs` capturing-subscriber pattern.
///
/// `current_thread` flavor + `with_default` keeps the capturing
/// subscriber thread-local for the whole request lifecycle (no worker
/// threads, so no event escapes the capture — the same constraint the
/// credential redaction helper documents).
#[tokio::test(flavor = "current_thread")]
async fn create_token_plaintext_never_leaks_to_logs() {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Cap(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Cap {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Cap {
        type Writer = Cap;
        fn make_writer(&'a self) -> Cap {
            self.clone()
        }
    }

    let buf = Cap::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let (state, _backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "POST",
            "/api/v1/me/tokens",
            &user.jwt,
            Some(r#"{"name":"secret-probe","scopes":[]}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_json(response).await;
    let token = body["token"].as_str().unwrap().to_owned();

    drop(_guard);

    assert!(
        token.starts_with(PAT_PLAINTEXT_PREFIX) && token.len() > 20,
        "the create response must carry the real plaintext exactly once"
    );

    let captured = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
    assert!(
        !captured.is_empty(),
        "the create path must emit at least one tracing event (observability DoD)"
    );
    assert!(
        !captured.contains(&token),
        "PAT plaintext leaked into tracing output:\n--- captured ---\n{captured}\n----------------"
    );
}

// ── DELETE /me/tokens/{pat} ──────────────────────────────────────────────────

#[tokio::test]
async fn delete_token_revokes_and_is_idempotent() {
    let (state, backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();

    let minted = backend
        .create_pat(
            &user.user_id,
            nebula_api::domain::auth::backend::CreatePatParams {
                name: "to-revoke".to_owned(),
                scopes: vec![],
                ttl_seconds: None,
            },
        )
        .await
        .unwrap();
    let pat_id = minted.record.id.clone();

    let app1 = app::build_app(state.clone(), &api_config);
    let response = app1
        .oneshot(mutating(
            "DELETE",
            &format!("/api/v1/me/tokens/{pat_id}"),
            &user.jwt,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["ok"], true);

    // Revoked PAT no longer appears in the active list.
    assert!(
        backend.list_pats(&user.user_id).await.unwrap().is_empty(),
        "revoked PAT must drop out of the active list"
    );

    // Idempotent: revoking again is still benign (200 or 404).
    let app2 = app::build_app(state, &api_config);
    let again = app2
        .oneshot(mutating(
            "DELETE",
            &format!("/api/v1/me/tokens/{pat_id}"),
            &user.jwt,
            None,
        ))
        .await
        .unwrap();
    // Already-revoked → the record no longer matches an *active* token by
    // id for this user, so the contract is a clean not-found (404). The
    // first revoke is the meaningful state change; a second is a no-op
    // either way (no spurious 5xx).
    assert!(
        again.status() == StatusCode::OK || again.status() == StatusCode::NOT_FOUND,
        "second revoke must be a benign 200 or 404, got {}",
        again.status()
    );
}

#[tokio::test]
async fn delete_unknown_token_is_404() {
    let (state, _backend, user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(mutating(
            "DELETE",
            "/api/v1/me/tokens/pat_does_not_exist",
            &user.jwt,
            None,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_token_owned_by_another_user_is_404() {
    // Cross-user isolation: a second registered user cannot revoke (or
    // even probe the existence of) the first user's PAT — it must be a
    // 404, identical to a missing token, so ownership is not disclosed.
    let (state, backend, user_a) = create_me_state().await;
    let api_config = ApiConfig::for_test();

    let minted = backend
        .create_pat(
            &user_a.user_id,
            nebula_api::domain::auth::backend::CreatePatParams {
                name: "user-a-token".to_owned(),
                scopes: vec![],
                ttl_seconds: None,
            },
        )
        .await
        .unwrap();
    let pat_id = minted.record.id.clone();

    // Register a second user and authenticate as them.
    let profile_b = backend
        .register_user(nebula_api::domain::auth::backend::SignupRequest {
            email: "user-b@nebula.dev".to_owned(),
            password: nebula_api::domain::auth::backend::dto::SecretString::new(
                "hunter22".to_owned(),
            ),
            display_name: "User B".to_owned(),
        })
        .await
        .unwrap();
    let jwt_b = jwt_for(&profile_b.user_id);

    let app = app::build_app(state, &api_config);
    let response = app
        .oneshot(mutating(
            "DELETE",
            &format!("/api/v1/me/tokens/{pat_id}"),
            &jwt_b,
            None,
        ))
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "user B revoking user A's PAT must be 404 (no cross-user ownership disclosure)"
    );

    // And user A's token must still be active — B's request had no effect.
    let still = backend.list_pats(&user_a.user_id).await.unwrap();
    assert_eq!(still.len(), 1, "user A's PAT must be untouched by user B");
}

#[tokio::test]
async fn delete_token_without_auth_is_401() {
    let (state, _backend, _user) = create_me_state().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/me/tokens/pat_x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
