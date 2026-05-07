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
    IDEMPOTENCY_KEY_HEADER, IDEMPOTENT_REPLAY_HEADER, IdempotencyConfig, IdempotencyLayer,
    InMemoryIdempotencyStore,
};
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_API_IDEMPOTENCY_HITS_TOTAL, NEBULA_API_IDEMPOTENCY_LATENCY_MS,
        NEBULA_API_IDEMPOTENCY_MISSES_TOTAL, NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL,
        NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM, idempotency_reject_reason,
    },
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

/// Build an app variant that wires `IdempotencyLayer::with_metrics(Some(registry))`
/// so the C2 metrics path is exercised end-to-end.
fn build_app_with_metrics(
    counter: Arc<AtomicUsize>,
    store: Arc<InMemoryIdempotencyStore>,
    registry: Arc<MetricsRegistry>,
) -> Router {
    let counter_for_post = counter.clone();
    let counter_for_failing = counter;

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
        .layer(IdempotencyLayer::new(store).with_metrics(Some(registry)))
}

#[tokio::test]
async fn metrics_hits_and_misses_increment() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let registry = Arc::new(MetricsRegistry::new());
    let app = build_app_with_metrics(counter, store, registry.clone());

    // First call → miss.
    let _ = app
        .clone()
        .oneshot(post_with_key("/echo", "metric-key", "x"))
        .await
        .unwrap();
    // Second call → hit.
    let _ = app
        .clone()
        .oneshot(post_with_key("/echo", "metric-key", "x"))
        .await
        .unwrap();

    let hits = registry
        .counter(NEBULA_API_IDEMPOTENCY_HITS_TOTAL)
        .unwrap()
        .get();
    let misses = registry
        .counter(NEBULA_API_IDEMPOTENCY_MISSES_TOTAL)
        .unwrap()
        .get();
    assert_eq!(hits, 1, "second call must record a hit");
    assert_eq!(misses, 1, "first call must record a miss");

    let histogram = registry
        .histogram(NEBULA_API_IDEMPOTENCY_LATENCY_MS)
        .unwrap();
    assert_eq!(
        histogram.count(),
        2,
        "latency histogram must observe one sample per request that reached the layer"
    );

    let saturation = registry
        .gauge(NEBULA_API_IDEMPOTENCY_STORE_SATURATION_PPM)
        .unwrap()
        .get();
    assert!(
        saturation >= 0,
        "saturation gauge must be set after a successful put"
    );
}

#[tokio::test]
async fn metrics_rejects_record_body_mismatch_label() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let registry = Arc::new(MetricsRegistry::new());
    let app = build_app_with_metrics(counter, store, registry.clone());

    // Prime the cache.
    let _ = app
        .clone()
        .oneshot(post_with_key("/echo", "mismatch-key", "first"))
        .await
        .unwrap();
    // Same key, different body → 422 + rejects[body_mismatch].
    let r2 = app
        .clone()
        .oneshot(post_with_key("/echo", "mismatch-key", "second"))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let labels = registry
        .interner()
        .single("reason", idempotency_reject_reason::BODY_MISMATCH);
    let rejects = registry
        .counter_labeled(NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL, &labels)
        .unwrap()
        .get();
    assert_eq!(
        rejects, 1,
        "body-mismatch must increment rejects[body_mismatch]"
    );
}

#[tokio::test]
async fn metrics_rejects_record_invalid_key_label() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    let registry = Arc::new(MetricsRegistry::new());
    let app = build_app_with_metrics(counter, store, registry.clone());

    // Empty key → reject:invalid_key (`IdempotencyKeyError::Empty`).
    let req = Request::builder()
        .method("POST")
        .uri("/echo")
        .header("content-type", "text/plain")
        .header(IDEMPOTENCY_KEY_HEADER, "")
        .body(Body::from("x"))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);

    let labels = registry
        .interner()
        .single("reason", idempotency_reject_reason::INVALID_KEY);
    let rejects = registry
        .counter_labeled(NEBULA_API_IDEMPOTENCY_REJECTS_TOTAL, &labels)
        .unwrap()
        .get();
    assert_eq!(
        rejects, 1,
        "empty Idempotency-Key must increment rejects[invalid_key]"
    );
}

/// Build the app fixture with a custom [`IdempotencyConfig`] so tests can
/// exercise the small-cap path on request and response bodies.
fn build_app_with_config(
    counter: Arc<AtomicUsize>,
    store: Arc<InMemoryIdempotencyStore>,
    config: IdempotencyConfig,
) -> Router {
    let counter_for_post = counter;

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
        .layer(IdempotencyLayer::new(store).with_config(config))
}

/// Regression: when the response body exceeds
/// `max_response_body_bytes`, the caller MUST receive the full body
/// from the inner handler (not an empty `Body::empty()`). The previous
/// implementation returned an empty body on overflow; PR #658 (Codex
/// P1) fixes this via a Content-Length pre-check.
#[tokio::test]
async fn oversized_response_body_returns_full_body_unchanged() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    // Cap responses at 4 bytes so the echo handler's "echo-1-x" output
    // (8 bytes) trips the pre-check.
    let config = IdempotencyConfig {
        max_request_body_bytes: 1024,
        max_response_body_bytes: 4,
    };
    let app = build_app_with_config(counter.clone(), store, config);

    let response = app
        .oneshot(post_with_key("/echo", "oversize", "x"))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert_eq!(
        body, "echo-1-x",
        "oversized response must forward the full inner-handler body, not an empty body \
         (regression: Codex P1 finding on PR #658)"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "inner handler must have run exactly once"
    );
}

/// Regression: when the request body exceeds
/// `max_request_body_bytes`, the inner handler MUST see the full
/// request body (not an empty body). The previous implementation
/// silently substituted `Body::empty()` on overflow; PR #658 fixes
/// this via a Content-Length pre-check.
#[tokio::test]
async fn oversized_request_body_passes_through_with_full_body() {
    let counter = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(InMemoryIdempotencyStore::new());
    // Cap requests at 4 bytes — the test payload "abcdef" is 6 bytes
    // and trips the pre-check.
    let config = IdempotencyConfig {
        max_request_body_bytes: 4,
        max_response_body_bytes: 1024,
    };
    let app = build_app_with_config(counter.clone(), store, config);

    let response = app
        .oneshot(post_with_key("/echo", "oversize-req", "abcdef"))
        .await
        .expect("request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert_eq!(
        body, "echo-1-abcdef",
        "oversized request body must reach the inner handler unchanged \
         (regression: Codex P1 finding on PR #658, request side)"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "inner handler must have run exactly once"
    );
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
