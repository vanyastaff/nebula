//! End-to-end tests for the idempotency middleware against the real
//! `build_app` router (M3.4 — ADR-0048).
//!
//! These tests differ from `tests/idempotency_middleware.rs` in two ways:
//!
//! 1. The router under test is the production router from
//!    `nebula_api::app::build_app`, including request-id, security
//!    headers, CORS, and the rest of the outer middleware stack — not a
//!    minimal `Router::new().route(...).layer(IdempotencyLayer::new(...))`
//!    fixture. This catches layer-ordering bugs that the minimal-router
//!    tests cannot.
//! 2. Test fixtures (`POST /api/v1/_test/echo`, `POST /api/v1/_test/fail`)
//!    are mounted via `axum::Router::merge` BEFORE the idempotency layer
//!    inside `build_app` (gated behind `feature = "test-util"`); they
//!    never appear in the served OpenAPI 3.1 spec (ADR-0047 honesty).
//!
//! Roadmap: §M3.4 Idempotency-Key dedup — fourth box (end-to-end test
//! against the real `build_app`).

mod common;

use std::sync::Arc;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use common::create_state_with_queue;
use nebula_api::{
    ApiConfig, AppState, app,
    middleware::idempotency::{
        IDEMPOTENCY_KEY_HEADER, IDEMPOTENT_REPLAY_HEADER, InMemoryIdempotencyStore,
    },
};
use tower::ServiceExt;

const X_REQUEST_ID: &str = "x-request-id";

/// Build an `AppState` with the in-memory idempotency store wired in.
///
/// Mirrors the dev composition-root pattern (`simple_server.rs`) modulo
/// the auth/RBAC layers — those run inside `build_app` but the test
/// echo route lives outside the OpenAPI router and is not subject to
/// them.
async fn state_with_idempotency() -> AppState {
    let (state, _queue) = create_state_with_queue().await;
    state.with_idempotency_store(Arc::new(InMemoryIdempotencyStore::new()))
}

fn post_with_key(uri: &str, key: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "text/plain")
        .header(IDEMPOTENCY_KEY_HEADER, key)
        .body(Body::from(body))
        .unwrap()
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn replay_returns_cached_body_through_full_stack() {
    let state = state_with_idempotency().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/api/v1/_test/echo", "k1", "alpha"))
        .await
        .expect("first request");
    assert_eq!(r1.status(), StatusCode::OK);
    let r1_request_id = r1.headers().get(X_REQUEST_ID).cloned();
    let r1_replay = r1.headers().get(IDEMPOTENT_REPLAY_HEADER).cloned();
    assert!(
        r1_replay.is_none(),
        "first call must NOT carry Idempotent-Replay"
    );
    let body1 = body_string(r1).await;

    let r2 = app
        .clone()
        .oneshot(post_with_key("/api/v1/_test/echo", "k1", "alpha"))
        .await
        .expect("second request");
    assert_eq!(r2.status(), StatusCode::OK);
    let r2_replay = r2
        .headers()
        .get(IDEMPOTENT_REPLAY_HEADER)
        .cloned()
        .expect("replay must carry Idempotent-Replay header");
    assert_eq!(r2_replay.to_str().unwrap(), "true");
    let r2_request_id = r2.headers().get(X_REQUEST_ID).cloned();
    let body2 = body_string(r2).await;

    assert_eq!(
        body1, body2,
        "second call must replay the same body byte-for-byte"
    );
    assert_eq!(
        body1, "echo:1:alpha",
        "echo handler must have run exactly once (counter == 1)"
    );

    // Layer-ordering proof: `request_id_middleware` sits OUTSIDE
    // idempotency, so a cached replay still acquires a fresh
    // `X-Request-ID`.
    let id_a = r1_request_id.expect("first call must have X-Request-ID");
    let id_b = r2_request_id.expect("replay must still get a fresh X-Request-ID");
    assert_ne!(
        id_a, id_b,
        "cached replays must receive a fresh X-Request-ID — request_id layer sits outside idempotency",
    );
}

#[tokio::test]
async fn body_mismatch_returns_422_through_full_stack() {
    let state = state_with_idempotency().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/api/v1/_test/echo", "k2", "first"))
        .await
        .expect("priming request");
    assert_eq!(r1.status(), StatusCode::OK);

    let r2 = app
        .clone()
        .oneshot(post_with_key("/api/v1/_test/echo", "k2", "second"))
        .await
        .expect("body-mismatch request");
    assert_eq!(
        r2.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "same Idempotency-Key with different body must be 422"
    );

    let content_type = r2
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("application/problem+json"),
        "422 must use RFC 9457 problem+json: got {content_type:?}"
    );
}

#[tokio::test]
async fn fivexx_response_is_not_cached_through_full_stack() {
    let state = state_with_idempotency().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/api/v1/_test/fail", "kf", "x"))
        .await
        .expect("first failing call");
    assert_eq!(r1.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body1 = body_string(r1).await;

    let r2 = app
        .clone()
        .oneshot(post_with_key("/api/v1/_test/fail", "kf", "x"))
        .await
        .expect("second failing call");
    assert_eq!(r2.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body2 = body_string(r2).await;

    assert_eq!(body1, "boom:1");
    assert_eq!(
        body2, "boom:2",
        "5xx must NOT be cached — handler must run again on a retried key (counter advances)"
    );
}

#[tokio::test]
async fn per_principal_scope_through_full_stack() {
    let state = state_with_idempotency().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let req_a = Request::builder()
        .method("POST")
        .uri("/api/v1/_test/echo")
        .header("content-type", "text/plain")
        .header("authorization", "Bearer caller-A")
        .header(IDEMPOTENCY_KEY_HEADER, "shared")
        .body(Body::from("payload"))
        .unwrap();
    let req_b = Request::builder()
        .method("POST")
        .uri("/api/v1/_test/echo")
        .header("content-type", "text/plain")
        .header("authorization", "Bearer caller-B")
        .header(IDEMPOTENCY_KEY_HEADER, "shared")
        .body(Body::from("payload"))
        .unwrap();

    let r_a = app.clone().oneshot(req_a).await.expect("caller A");
    let r_b = app.clone().oneshot(req_b).await.expect("caller B");

    assert_eq!(r_a.status(), StatusCode::OK);
    assert_eq!(r_b.status(), StatusCode::OK);
    let body_a = body_string(r_a).await;
    let body_b = body_string(r_b).await;
    assert_eq!(body_a, "echo:1:payload");
    assert_eq!(
        body_b, "echo:2:payload",
        "two callers using the same Idempotency-Key must NOT share cache entries — \
         identity-fingerprint must scope the cache per principal even through the \
         full middleware stack"
    );
}

#[tokio::test]
async fn cors_allows_idempotency_key_in_preflight() {
    let state = state_with_idempotency().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let preflight = Request::builder()
        .method("OPTIONS")
        .uri("/api/v1/_test/echo")
        .header("origin", "https://app.nebula.dev")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "idempotency-key")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(preflight).await.expect("preflight");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "preflight with Idempotency-Key request-header must succeed"
    );

    let allow_headers = response
        .headers()
        .get("access-control-allow-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        allow_headers.contains("idempotency-key"),
        "Access-Control-Allow-Headers must list `idempotency-key`: got {allow_headers:?}",
    );
}

#[tokio::test]
async fn cors_exposes_idempotent_replay_header() {
    // Per the CORS spec, `Access-Control-Expose-Headers` is set on the
    // **actual** cross-origin response (not the preflight); browsers strip
    // any non-listed response header before `fetch().headers` sees it. So
    // assert against a real POST with an `Origin` header rather than the
    // OPTIONS preflight.
    let state = state_with_idempotency().await;
    let api_config = ApiConfig::for_test();
    let app = app::build_app(state, &api_config);

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/_test/echo")
        .header("origin", "https://app.nebula.dev")
        .header("content-type", "text/plain")
        .header(IDEMPOTENCY_KEY_HEADER, "expose-test")
        .body(Body::from("ping"))
        .unwrap();

    let response = app.oneshot(request).await.expect("cross-origin POST");
    assert_eq!(response.status(), StatusCode::OK);

    let expose_headers = response
        .headers()
        .get("access-control-expose-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        expose_headers.contains("idempotent-replay"),
        "Access-Control-Expose-Headers must list `idempotent-replay` on actual cross-origin responses: got {expose_headers:?}",
    );
}
