//! Integration tests for the Idempotency-Key middleware (M3.4).
//!
//! These tests exercise the middleware against a minimal in-test
//! `Router` so the assertions stay focused on idempotency semantics
//! without dragging in auth, CSRF, or tenant-scope concerns from the
//! production stack.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use nebula_api::middleware::idempotency::{
    IDEMPOTENCY_KEY_HEADER, IDEMPOTENT_REPLAY_HEADER, IdempotencyLayer, InMemoryIdempotencyStore,
};
use tower::ServiceExt;

/// Build a tiny test app whose POST handler echoes its payload, suffixed with
/// a monotonically increasing call counter. The counter exposes whether the
/// inner handler ran (idempotency miss) or was bypassed (idempotency hit).
fn build_app(counter: Arc<AtomicUsize>, store: Arc<InMemoryIdempotencyStore>) -> Router {
    let counter_for_post = counter.clone();
    let counter_for_failing = counter.clone();
    let counter_for_get = counter;

    Router::new()
        .route(
            "/echo",
            post(move |body: axum::body::Bytes| {
                let counter = counter_for_post.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    let payload = String::from_utf8_lossy(&body);
                    (StatusCode::OK, format!("echo-{n}-{payload}")).into_response()
                }
            }),
        )
        .route(
            "/fail",
            post(move || {
                let counter = counter_for_failing.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
                }
            }),
        )
        .route(
            "/peek",
            get(move || {
                let counter = counter_for_get.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::OK, "peek").into_response()
                }
            }),
        )
        .layer(IdempotencyLayer::new(store))
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

fn post_without_key(uri: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "text/plain")
        .body(Body::from(body))
        .unwrap()
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn replayed_key_returns_cached_body_and_skips_handler() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/echo", "key-1", "hello"))
        .await
        .expect("first request");
    assert_eq!(r1.status(), StatusCode::OK);
    let replay_marker_r1 = r1.headers().get(IDEMPOTENT_REPLAY_HEADER).cloned();
    let body1 = body_string(r1).await;
    assert_eq!(body1, "echo-1-hello");
    assert!(
        replay_marker_r1.is_none(),
        "first call must not be marked as a replay"
    );

    let r2 = app
        .clone()
        .oneshot(post_with_key("/echo", "key-1", "hello"))
        .await
        .expect("second request");
    assert_eq!(r2.status(), StatusCode::OK);
    let replay_marker_r2 = r2.headers().get(IDEMPOTENT_REPLAY_HEADER).cloned();
    let body2 = body_string(r2).await;
    assert_eq!(
        body2, "echo-1-hello",
        "replayed body must match the cached first response (counter frozen at 1)"
    );
    assert_eq!(
        replay_marker_r2.as_ref().and_then(|v| v.to_str().ok()),
        Some("true"),
        "cache hit must set the {IDEMPOTENT_REPLAY_HEADER} header"
    );

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "inner handler must run exactly once across two key replays"
    );
}

#[tokio::test]
async fn distinct_keys_run_handler_each_time() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/echo", "key-A", "x"))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    assert_eq!(body_string(r1).await, "echo-1-x");

    let r2 = app
        .clone()
        .oneshot(post_with_key("/echo", "key-B", "x"))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(
        body_string(r2).await,
        "echo-2-x",
        "different key must produce a fresh response"
    );

    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn missing_idempotency_header_passes_through() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    let r1 = app
        .clone()
        .oneshot(post_without_key("/echo", "n"))
        .await
        .unwrap();
    let r2 = app
        .clone()
        .oneshot(post_without_key("/echo", "n"))
        .await
        .unwrap();

    assert_eq!(r1.status(), StatusCode::OK);
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(body_string(r1).await, "echo-1-n");
    assert_eq!(
        body_string(r2).await,
        "echo-2-n",
        "without the header, every POST must invoke the handler",
    );
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn non_post_methods_pass_through() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    for _ in 0..2 {
        let req = Request::builder()
            .method("GET")
            .uri("/peek")
            .header(IDEMPOTENCY_KEY_HEADER, "ignored-key")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let _ = body_string(resp).await;
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "GET must not engage the idempotency cache",
    );
}

#[tokio::test]
async fn same_key_with_different_body_returns_422() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/echo", "key-collide", "alpha"))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let _ = body_string(r1).await;

    let r2 = app
        .clone()
        .oneshot(post_with_key("/echo", "key-collide", "beta"))
        .await
        .unwrap();
    assert_eq!(
        r2.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "reusing an Idempotency-Key with a different body must yield 422"
    );
    let body = body_string(r2).await;
    assert!(
        body.contains("different request payload"),
        "422 body should explain the conflict, got: {body}"
    );

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "handler must run only for the first request — the conflicting replay is rejected pre-dispatch",
    );
}

#[tokio::test]
async fn malformed_idempotency_key_returns_400() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    // A tab character is outside the printable-ASCII range allowed by
    // `IdempotencyKey::parse`. The middleware MUST reject it before the
    // handler runs.
    let req = Request::builder()
        .method("POST")
        .uri("/echo")
        .header("content-type", "text/plain")
        .header(IDEMPOTENCY_KEY_HEADER, "bad\tkey")
        .body(Body::from("payload"))
        .unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "handler must not run when the key fails validation",
    );
}

#[tokio::test]
async fn fivexx_responses_are_not_cached() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/fail", "transient", ""))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let r2 = app
        .clone()
        .oneshot(post_with_key("/fail", "transient", ""))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::INTERNAL_SERVER_ERROR);

    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "5xx responses must remain uncached so a transient backend failure can be retried"
    );
}

#[tokio::test]
async fn cache_entry_expires_after_ttl() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::with_ttl_and_capacity(
        Duration::from_millis(80),
        64,
    ));
    let app = build_app(counter.clone(), store);

    let r1 = app
        .clone()
        .oneshot(post_with_key("/echo", "ttl-key", "p"))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let _ = body_string(r1).await;

    // Wait past the TTL — moka may need a tick to evict, but a sleep > TTL
    // followed by a get triggers eviction without explicit run_pending_tasks
    // here because the API surface used in production does not expose it.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let r2 = app
        .clone()
        .oneshot(post_with_key("/echo", "ttl-key", "p"))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(
        body_string(r2).await,
        "echo-2-p",
        "after TTL expiry, the same key must produce a fresh response (counter advances)"
    );
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn distinct_identities_do_not_share_cache_entries() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let app = build_app(counter.clone(), store);

    let req_a = Request::builder()
        .method("POST")
        .uri("/echo")
        .header("content-type", "text/plain")
        .header("authorization", "Bearer caller-A")
        .header(IDEMPOTENCY_KEY_HEADER, "shared")
        .body(Body::from("payload"))
        .unwrap();
    let req_b = Request::builder()
        .method("POST")
        .uri("/echo")
        .header("content-type", "text/plain")
        .header("authorization", "Bearer caller-B")
        .header(IDEMPOTENCY_KEY_HEADER, "shared")
        .body(Body::from("payload"))
        .unwrap();

    let r_a = app.clone().oneshot(req_a).await.unwrap();
    let r_b = app.clone().oneshot(req_b).await.unwrap();

    assert_eq!(r_a.status(), StatusCode::OK);
    assert_eq!(r_b.status(), StatusCode::OK);
    assert_eq!(body_string(r_a).await, "echo-1-payload");
    assert_eq!(
        body_string(r_b).await,
        "echo-2-payload",
        "two callers using the same Idempotency-Key must NOT share cache entries — \
         identity-fingerprint must scope the cache per principal",
    );
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}
