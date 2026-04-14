//! Integration tests for `nebula-api::webhook::WebhookTransport`.
//!
//! Exercise the full HTTP round-trip: build a real axum router from
//! the transport, drive requests through it with
//! `tower::ServiceExt::oneshot`, and assert that:
//!
//! - valid signed requests reach the action and emit
//! - bad signatures skip cleanly (200 with no workflow emission)
//! - `ctx.webhook.endpoint_url()` is populated at activation time
//! - unknown `(trigger_uuid, nonce)` returns 404
//! - oversized bodies return 413
//! - rate limiting returns 429
//! - handler timeout returns 504
//!
//! These tests use the `WebhookTransport` in isolation (they do NOT
//! build the full `nebula-api` app). That keeps the surface small
//! and the failure modes easy to read.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    http::{Request, StatusCode},
};
use hmac::{Hmac, KeyInit, Mac};
use nebula_action::{
    Action, ActionDependencies, ActionError, ActionMetadata, TriggerContext, TriggerEventOutcome,
    TriggerHandler, WebhookAction, WebhookRequest, WebhookResponse, WebhookTriggerAdapter,
};
use nebula_api::webhook::{WebhookTransport, WebhookTransportConfig};
use sha2::Sha256;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use url::Url;

type HmacSha256 = Hmac<Sha256>;

// ── Shared test helpers ──────────────────────────────────────────────────

struct GitHubLikeWebhook {
    meta: ActionMetadata,
    secret: Vec<u8>,
    captured_url: Arc<Mutex<Option<Url>>>,
}

#[derive(Clone)]
#[allow(dead_code)] // hook_id is set to simulate a real registration; tests only read the URL
struct RegistrationState {
    hook_id: u64,
}

impl ActionDependencies for GitHubLikeWebhook {}
impl Action for GitHubLikeWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for GitHubLikeWebhook {
    type State = RegistrationState;

    async fn on_activate(&self, ctx: &TriggerContext) -> Result<Self::State, ActionError> {
        // Capture the URL that the transport injected via the
        // WebhookEndpointProvider capability. A real action would
        // call the provider API to register a hook pointing at this
        // URL; we just stash it for assertion.
        let endpoint = ctx
            .webhook
            .as_ref()
            .ok_or_else(|| ActionError::fatal("webhook endpoint provider missing from ctx"))?;
        *self.captured_url.lock().unwrap() = Some(endpoint.endpoint_url().clone());
        Ok(RegistrationState { hook_id: 12345 })
    }

    async fn handle_request(
        &self,
        request: &WebhookRequest,
        _state: &RegistrationState,
        _ctx: &TriggerContext,
    ) -> Result<WebhookResponse, ActionError> {
        let outcome =
            nebula_action::verify_hmac_sha256(request, &self.secret, "X-Hub-Signature-256")?;
        if !outcome.is_valid() {
            return Ok(WebhookResponse::accept(TriggerEventOutcome::skip()));
        }
        let payload = request
            .body_json::<serde_json::Value>()
            .map_err(|e| ActionError::fatal(format!("bad json: {e}")))?;
        Ok(WebhookResponse::accept(TriggerEventOutcome::emit(payload)))
    }

    async fn on_deactivate(
        &self,
        _state: RegistrationState,
        _ctx: &TriggerContext,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}

fn sign(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn make_transport(rate_limit: Option<u64>, body_limit: usize) -> WebhookTransport {
    WebhookTransport::new(WebhookTransportConfig {
        base_url: Url::parse("https://nebula.example.com").unwrap(),
        path_prefix: "/webhooks".to_string(),
        body_limit_bytes: body_limit,
        response_timeout: Duration::from_secs(5),
        rate_limit_per_minute: rate_limit,
    })
}

async fn register_webhook(
    transport: &WebhookTransport,
    secret: Vec<u8>,
) -> (
    nebula_api::webhook::ActivationHandle,
    Arc<Mutex<Option<Url>>>,
) {
    let captured = Arc::new(Mutex::new(None));
    let webhook = GitHubLikeWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.integration"),
            "GitHub-like",
            "Integration test webhook",
        ),
        secret,
        captured_url: captured.clone(),
    };
    let adapter: Arc<dyn TriggerHandler> = Arc::new(WebhookTriggerAdapter::new(webhook));

    let ctx_template = TriggerContext::new(
        nebula_core::WorkflowId::new(),
        nebula_core::NodeId::new(),
        CancellationToken::new(),
    );

    let handle = transport.activate(adapter.clone(), ctx_template).unwrap();
    // Simulate runtime calling adapter.start() after activation
    // wired the endpoint capability into the context.
    adapter.start(&handle.ctx).await.unwrap();
    (handle, captured)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn on_activate_sees_endpoint_url_from_context() {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, captured) = register_webhook(&transport, b"secret".to_vec()).await;
    let url = captured
        .lock()
        .unwrap()
        .clone()
        .expect("url must be captured");
    assert_eq!(url, handle.endpoint_url);
    assert!(
        url.as_str()
            .starts_with("https://nebula.example.com/webhooks/")
    );
    // Path has three components after the prefix: uuid and nonce.
    let path_segments: Vec<&str> = url.path_segments().unwrap().collect();
    assert_eq!(path_segments[0], "webhooks");
    // uuid + nonce
    assert_eq!(path_segments.len(), 3);
}

#[tokio::test]
async fn valid_signed_request_returns_200() {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();
    let body_bytes = br#"{"action":"opened","number":1}"#.to_vec();
    let sig = sign(b"secret", &body_bytes);

    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .header("content-type", "application/json")
        .header("x-hub-signature-256", sig)
        .body(Body::from(body_bytes))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn bad_signature_returns_200_with_no_emission() {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .header("x-hub-signature-256", "sha256=deadbeef")
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    // Action returns WebhookResponse::accept(Skip) on bad sig →
    // HTTP 200 with no workflow emission. This IS the expected
    // behaviour: multi-tenant endpoints should not reveal which
    // trigger owns a given path to the external caller.
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_trigger_uuid_returns_404() {
    let transport = make_transport(None, 1024 * 1024);
    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri("/webhooks/550e8400-e29b-41d4-a716-446655440000/deadbeef")
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn malformed_uuid_in_path_returns_404() {
    let transport = make_transport(None, 1024 * 1024);
    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri("/webhooks/not-a-uuid/nonce")
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oversized_body_returns_413() {
    // 1 KiB body limit
    let transport = make_transport(None, 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();
    let big_body: Vec<u8> = vec![b'x'; 4096];
    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from(Bytes::from(big_body)))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rate_limit_returns_429_with_retry_after() {
    // 2 requests per minute, so the 3rd will trip the limiter.
    let transport = make_transport(Some(2), 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();

    let make_request = || {
        Request::builder()
            .method("POST")
            .uri(handle.endpoint_url.path())
            .body(Body::from("{}"))
            .unwrap()
    };

    // First two pass (bad signature → 200 with Skip, but through
    // the limiter).
    let r1 = router.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let r2 = router.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(r2.status(), StatusCode::OK);

    // Third trips the limit.
    let r3 = router.clone().oneshot(make_request()).await.unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        r3.headers().get("retry-after").is_some(),
        "429 must include Retry-After header"
    );
}

// ── Handler-timeout test with a hanging action ──────────────────────────

struct HangingWebhook {
    meta: ActionMetadata,
}

impl ActionDependencies for HangingWebhook {}
impl Action for HangingWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for HangingWebhook {
    type State = ();

    async fn on_activate(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &TriggerContext,
    ) -> Result<WebhookResponse, ActionError> {
        std::future::pending::<()>().await;
        unreachable!()
    }
}

#[tokio::test]
async fn handler_timeout_returns_504() {
    let transport = WebhookTransport::new(WebhookTransportConfig {
        base_url: Url::parse("https://nebula.example.com").unwrap(),
        path_prefix: "/webhooks".to_string(),
        body_limit_bytes: 1024 * 1024,
        response_timeout: Duration::from_millis(200),
        rate_limit_per_minute: None,
    });

    let adapter: Arc<dyn TriggerHandler> = Arc::new(WebhookTriggerAdapter::new(HangingWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.hang"),
            "Hanging",
            "Handler that never returns",
        ),
    }));
    let ctx_template = TriggerContext::new(
        nebula_core::WorkflowId::new(),
        nebula_core::NodeId::new(),
        CancellationToken::new(),
    );
    let handle = transport.activate(adapter.clone(), ctx_template).unwrap();
    adapter.start(&handle.ctx).await.unwrap();

    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn deactivate_removes_route() {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();
    let path = handle.endpoint_url.path().to_string();

    // Pre-deactivate: route is live, valid request → 200.
    let body_bytes = br#"{"ok":1}"#.to_vec();
    let sig = sign(b"secret", &body_bytes);
    let req = Request::builder()
        .method("POST")
        .uri(&path)
        .header("x-hub-signature-256", sig)
        .body(Body::from(body_bytes))
        .unwrap();
    let r = router.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    // Deactivate.
    transport.deactivate(&handle);

    // Post-deactivate: same path → 404.
    let req2 = Request::builder()
        .method("POST")
        .uri(&path)
        .body(Body::from("{}"))
        .unwrap();
    let r2 = router.clone().oneshot(req2).await.unwrap();
    assert_eq!(r2.status(), StatusCode::NOT_FOUND);
}

// ── #312: webhook router must only accept POST ──────────────────────────
//
// Regression tests for issue #312: the previous routing used
// `.route(&route, any(...))` which dispatched GET/PUT/DELETE/PATCH
// through the handler. The fix changes that to `.route(..., post(...))`
// so axum auto-returns 405 with `Allow: POST` for non-POST methods.

/// Helper: build an activated transport + the path under which its
/// webhook is registered.
async fn make_activated_transport() -> (WebhookTransport, String) {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;
    let path = handle.endpoint_url.path().to_string();
    (transport, path)
}

async fn assert_method_rejected(method: &'static str) {
    let (transport, path) = make_activated_transport().await;
    let router = transport.router();

    let request = Request::builder()
        .method(method)
        .uri(&path)
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "{method} must be rejected with 405"
    );
    let allow = response
        .headers()
        .get("allow")
        .expect("405 must include Allow header")
        .to_str()
        .unwrap()
        .to_ascii_uppercase();
    assert!(
        allow.contains("POST"),
        "Allow header must advertise POST, got: {allow}"
    );
}

#[tokio::test]
async fn rejects_get_with_405() {
    assert_method_rejected("GET").await;
}

#[tokio::test]
async fn rejects_put_with_405() {
    assert_method_rejected("PUT").await;
}

#[tokio::test]
async fn rejects_delete_with_405() {
    assert_method_rejected("DELETE").await;
}

#[tokio::test]
async fn rejects_patch_with_405() {
    assert_method_rejected("PATCH").await;
}

/// Regression lock: POST must still dispatch through the handler
/// after the method-gate change.
#[tokio::test]
async fn post_still_dispatches() {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();
    let body_bytes = br#"{"ping":true}"#.to_vec();
    let sig = sign(b"secret", &body_bytes);

    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .header("content-type", "application/json")
        .header("x-hub-signature-256", sig)
        .body(Body::from(body_bytes))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
