//! Integration tests for the slug-routed webhook dispatch path
//! (M3.3, `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}`).
//!
//! Wires the real axum router (`routes::webhook::router_with_dispatcher`)
//! with a test [`MpscSink`] and asserts each acceptance criterion from
//! the M3.3 task brief:
//!
//! 1. Validate per-trigger auth → enqueue → return 202 Accepted.
//! 2. Engine receives the trigger event (via the mpsc sink).
//! 3. Typed errors (`WebhookAuthError`, `WebhookEnqueueError`) map to correct HTTP status codes.
//!
//! Tests build a small router with a minimal [`AppState`] (in-memory
//! repos + test JWT secret). The handler still reads
//! `Extension<Arc<WebhookDispatcher>>` for dispatch; `AppState` is
//! required by the extractor but unused on this path.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use nebula_action::WebhookRequest;
use nebula_api::{
    config::ApiConfig,
    routes::webhook::router_with_dispatcher,
    state::AppState,
    webhook::{
        DEFAULT_SIGNATURE_HEADER, MpscSink, TriggerCoordinates, TriggerRegistration,
        WebhookAuthConfig, WebhookDispatcher, WebhookTriggerSink,
    },
};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};
use sha2::Sha256;
use tower::ServiceExt;

type HmacSha256 = Hmac<Sha256>;

const TEST_PATH: &str = "/api/v1/hooks/acme/main/github";

fn coords() -> TriggerCoordinates {
    TriggerCoordinates::new("acme", "main", "github")
}

fn test_app_state() -> AppState {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let config = ApiConfig::for_test();
    AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        config.jwt_secret,
    )
}

/// Same nesting + state shape as [`nebula_api::routes::create_routes`]: webhook
/// routes live under `/api/v1`, and `with_state` on the outer router satisfies
/// axum's `Service` bounds for `tower::ServiceExt::oneshot`.
fn build_router(dispatcher: Arc<WebhookDispatcher>) -> Router {
    let state = test_app_state();
    Router::new()
        .nest("/api/v1", router_with_dispatcher(dispatcher))
        .with_state(state)
}

fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn unsigned_public_webhook_returns_202_and_engine_receives_event() {
    let sink = Arc::new(MpscSink::new(2));
    let mut rx = sink.take_receiver().await.expect("rx available");

    let dispatcher = Arc::new(
        WebhookDispatcher::new()
            .with_default_sink(Arc::clone(&sink) as Arc<dyn WebhookTriggerSink>),
    );
    dispatcher
        .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
        .expect("first registration");

    let router = build_router(Arc::clone(&dispatcher));

    let body = br#"{"event":"push","ref":"main"}"#.to_vec();
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .header("content-type", "application/json")
        .body(Body::from(body.clone()))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "successful dispatch must return 202 Accepted",
    );

    // The engine sink must receive the trigger event within a
    // reasonable time.
    let dispatched = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("sink delivery timed out")
        .expect("sink dropped event");
    assert_eq!(dispatched.coordinates, coords());

    let (_id, _received_at, request) = dispatched
        .event
        .downcast::<WebhookRequest>()
        .expect("payload is WebhookRequest");
    assert_eq!(
        request.body(),
        body.as_slice(),
        "engine receives the original request body",
    );
}

#[tokio::test]
async fn signed_webhook_passes_auth_and_engine_sees_payload() {
    let secret: Arc<[u8]> = Arc::<[u8]>::from(b"top-secret".as_slice());
    let body = br#"{"hello":"hmac"}"#.to_vec();
    let signature = sign(&secret, &body);

    let sink = Arc::new(MpscSink::new(2));
    let mut rx = sink.take_receiver().await.unwrap();

    let dispatcher = Arc::new(
        WebhookDispatcher::new()
            .with_default_sink(Arc::clone(&sink) as Arc<dyn WebhookTriggerSink>),
    );
    dispatcher
        .register(
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::hmac_sha256(secret)),
        )
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .header("content-type", "application/json")
        .header(DEFAULT_SIGNATURE_HEADER, signature)
        .body(Body::from(body.clone()))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let dispatched = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("dispatch timed out")
        .unwrap();
    let (_id, _at, request) = dispatched
        .event
        .downcast::<WebhookRequest>()
        .expect("WebhookRequest payload");
    assert_eq!(request.body(), body.as_slice());
}

#[tokio::test]
async fn unsigned_request_to_signed_trigger_returns_401() {
    let secret: Arc<[u8]> = Arc::<[u8]>::from(b"top-secret".as_slice());
    let sink = Arc::new(MpscSink::new(1));
    let mut rx = sink.take_receiver().await.unwrap();

    let dispatcher = Arc::new(
        WebhookDispatcher::new()
            .with_default_sink(Arc::clone(&sink) as Arc<dyn WebhookTriggerSink>),
    );
    dispatcher
        .register(
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::hmac_sha256(secret)),
        )
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "missing signature → 401, not 202",
    );

    // Critical invariant: auth failure must not leak through to the
    // engine. The mpsc must still be empty.
    assert!(
        rx.try_recv().is_err(),
        "auth-rejected request must not reach the engine sink",
    );
}

#[tokio::test]
async fn unregistered_trigger_returns_404() {
    let dispatcher = Arc::new(WebhookDispatcher::new());
    let router = build_router(Arc::clone(&dispatcher));

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/hooks/acme/main/unknown")
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bearer_token_valid_returns_202() {
    let sink = Arc::new(MpscSink::new(1));
    let mut rx = sink.take_receiver().await.unwrap();
    let dispatcher = Arc::new(
        WebhookDispatcher::new()
            .with_default_sink(Arc::clone(&sink) as Arc<dyn WebhookTriggerSink>),
    );
    dispatcher
        .register(
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::bearer("my-shared-secret")),
        )
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .header("authorization", "Bearer my-shared-secret")
        .body(Body::from(b"{\"ok\":true}".to_vec()))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let _delivered = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("delivery within 2s")
        .unwrap();
}

#[tokio::test]
async fn bearer_token_wrong_returns_401() {
    let dispatcher = Arc::new(WebhookDispatcher::new());
    dispatcher
        .register(
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::bearer("the-real-token")),
        )
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .header("authorization", "Bearer not-the-real-token")
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn empty_secret_on_signed_trigger_returns_500() {
    // Operator misconfiguration: declared HMAC but secret is empty.
    let dispatcher = Arc::new(WebhookDispatcher::new());
    dispatcher
        .register(
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::hmac_sha256(Arc::<[u8]>::from(
                Vec::<u8>::new(),
            ))),
        )
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "missing secret is operator-side fail-closed (500)",
    );
}

#[tokio::test]
async fn closed_sink_returns_500() {
    let sink = Arc::new(MpscSink::new(1));
    // Take rx and immediately drop it to simulate engine shutdown.
    drop(sink.take_receiver().await.unwrap());

    let dispatcher = Arc::new(
        WebhookDispatcher::new()
            .with_default_sink(Arc::clone(&sink) as Arc<dyn WebhookTriggerSink>),
    );
    dispatcher
        .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));
    let request = Request::builder()
        .method("POST")
        .uri(TEST_PATH)
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn full_sink_returns_503() {
    let sink = Arc::new(MpscSink::new(1));
    // Don't take the receiver — buffer fills up to capacity 1.
    let dispatcher = Arc::new(
        WebhookDispatcher::new()
            .with_default_sink(Arc::clone(&sink) as Arc<dyn WebhookTriggerSink>),
    );
    dispatcher
        .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));

    // First request fills the channel.
    let r1 = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(TEST_PATH)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::ACCEPTED);

    // Second hits backpressure → 503.
    let r2 = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(TEST_PATH)
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn multiple_triggers_route_to_their_own_sinks() {
    let sink_gh = Arc::new(MpscSink::new(2));
    let mut rx_gh = sink_gh.take_receiver().await.unwrap();
    let sink_stripe = Arc::new(MpscSink::new(2));
    let mut rx_stripe = sink_stripe.take_receiver().await.unwrap();

    let dispatcher = Arc::new(WebhookDispatcher::new());
    dispatcher
        .register(
            TriggerCoordinates::new("acme", "main", "github"),
            TriggerRegistration::with_sink(
                WebhookAuthConfig::None,
                Arc::clone(&sink_gh) as Arc<dyn WebhookTriggerSink>,
            ),
        )
        .unwrap();
    dispatcher
        .register(
            TriggerCoordinates::new("acme", "main", "stripe"),
            TriggerRegistration::with_sink(
                WebhookAuthConfig::None,
                Arc::clone(&sink_stripe) as Arc<dyn WebhookTriggerSink>,
            ),
        )
        .unwrap();

    let router = build_router(Arc::clone(&dispatcher));

    let r_gh = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/acme/main/github")
                .body(Body::from(br#"{"trigger":"gh"}"#.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_gh.status(), StatusCode::ACCEPTED);

    let r_stripe = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/acme/main/stripe")
                .body(Body::from(br#"{"trigger":"stripe"}"#.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_stripe.status(), StatusCode::ACCEPTED);

    let gh_event = tokio::time::timeout(Duration::from_secs(2), rx_gh.recv())
        .await
        .expect("gh delivery")
        .unwrap();
    assert_eq!(gh_event.coordinates.trigger, "github");

    let stripe_event = tokio::time::timeout(Duration::from_secs(2), rx_stripe.recv())
        .await
        .expect("stripe delivery")
        .unwrap();
    assert_eq!(stripe_event.coordinates.trigger, "stripe");
}
