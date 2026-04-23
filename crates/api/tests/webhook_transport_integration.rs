//! Integration tests for `nebula-api::services::webhook::WebhookTransport`.
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
    Action, ActionError, ActionMetadata, SignaturePolicy, TriggerContext, TriggerEventOutcome,
    TriggerHandler, TriggerRuntimeContext, WebhookAction, WebhookConfig, WebhookRequest,
    WebhookResponse, WebhookTriggerAdapter,
};
use nebula_api::services::webhook::{WebhookTransport, WebhookTransportConfig};
use nebula_core::DeclaresDependencies;
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
#[expect(
    dead_code,
    reason = "hook_id is set to simulate a real registration; tests only read the URL"
)]
struct RegistrationState {
    hook_id: u64,
}

impl DeclaresDependencies for GitHubLikeWebhook {}
impl Action for GitHubLikeWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for GitHubLikeWebhook {
    type State = RegistrationState;

    fn config(&self) -> WebhookConfig {
        // Declare the signature policy at the trait surface so the
        // transport enforces it before dispatch (ADR-0022). In-handler
        // verification stays as defence-in-depth but the transport
        // would reject a mismatch first.
        WebhookConfig::default().with_signature_policy(
            SignaturePolicy::required_hmac_sha256(Arc::<[u8]>::from(self.secret.clone()))
                .clone_with_github_header(),
        )
    }

    async fn on_activate(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<Self::State, ActionError> {
        // Capture the URL that the transport injected via the
        // WebhookEndpointProvider capability. A real action would
        // call the provider API to register a hook pointing at this
        // URL; we just stash it for assertion.
        let endpoint = ctx
            .webhook_endpoint()
            .ok_or_else(|| ActionError::fatal("webhook endpoint provider missing from ctx"))?;
        *self.captured_url.lock().unwrap() = Some(endpoint.endpoint_url().clone());
        Ok(RegistrationState { hook_id: 12345 })
    }

    async fn handle_request(
        &self,
        request: &WebhookRequest,
        _state: &RegistrationState,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        // Defence-in-depth: the transport has already verified the
        // signature under our `config()` policy, but re-verifying
        // inside the handler would still be correct — there is no
        // in-handler path that could observe an unverified body.
        let payload = request
            .body_json::<serde_json::Value>()
            .map_err(|e| ActionError::fatal(format!("bad json: {e}")))?;
        Ok(WebhookResponse::accept(TriggerEventOutcome::emit(payload)))
    }

    async fn on_deactivate(
        &self,
        _state: RegistrationState,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }
}

/// Test helper: `SignaturePolicy::required_hmac_sha256(...)` defaults
/// to the canonical Nebula header (`X-Nebula-Signature`). Our GitHub-
/// like fixture uses `X-Hub-Signature-256` to keep the existing HTTP
/// request-building code unchanged. This trait extension keeps the
/// overriding boilerplate off the production API.
trait SignaturePolicyTestExt {
    fn clone_with_github_header(self) -> SignaturePolicy;
}

impl SignaturePolicyTestExt for SignaturePolicy {
    fn clone_with_github_header(self) -> SignaturePolicy {
        match self {
            SignaturePolicy::Required(req) => SignaturePolicy::Required(
                req.with_header_str("X-Hub-Signature-256")
                    .expect("static header name"),
            ),
            other => other,
        }
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
    nebula_api::services::webhook::ActivationHandle,
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
    let adapter = WebhookTriggerAdapter::new(webhook);
    // Read the cached webhook config BEFORE erasing the adapter
    // to `Arc<dyn TriggerHandler>` — webhook configuration does not
    // flow through the dyn trigger contract (ADR-0022).
    let config = adapter.config().clone();
    let adapter: Arc<dyn TriggerHandler> = Arc::new(adapter);

    let ctx_template = TriggerRuntimeContext::new(
        Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(CancellationToken::new())
                .build(),
        ),
        nebula_core::WorkflowId::new(),
        nebula_core::node_key!("test"),
    );

    let handle = transport
        .activate(adapter.clone(), config, ctx_template)
        .unwrap();
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
async fn bad_signature_returns_401_problem_json() {
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
    // ADR-0022: transport rejects at the boundary — the handler is
    // never given the chance to observe an unsigned / bad-signed
    // request. RFC 9457 problem+json carries the diagnostic.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json"),
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        problem["type"],
        "https://nebula.dev/problems/webhook-signature"
    );
    assert_eq!(problem["status"], 401);
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|s| s.contains("invalid")),
        "detail must identify invalid signature, got: {}",
        problem["detail"]
    );
}

#[tokio::test]
async fn missing_signature_header_returns_401_missing_reason() {
    let transport = make_transport(None, 1024 * 1024);
    let (handle, _) = register_webhook(&transport, b"secret".to_vec()).await;

    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|s| s.contains("missing")),
        "detail must identify missing header, got: {}",
        problem["detail"]
    );
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
    // The rate limiter runs before signature enforcement (ADR-0022
    // only governs dispatch, not ingress shaping), so any 3+ request
    // trips 429 regardless of signature state.
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

    // First two requests: unsigned → 401 under ADR-0022. But they pass
    // *through* the rate limiter — we only assert that they do not
    // trip the 429 path, not the specific status.
    let r1 = router.clone().oneshot(make_request()).await.unwrap();
    assert_ne!(r1.status(), StatusCode::TOO_MANY_REQUESTS);
    let r2 = router.clone().oneshot(make_request()).await.unwrap();
    assert_ne!(r2.status(), StatusCode::TOO_MANY_REQUESTS);

    // Third trips the limit before the signature stage is reached.
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

impl DeclaresDependencies for HangingWebhook {}
impl Action for HangingWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for HangingWebhook {
    type State = ();

    fn config(&self) -> WebhookConfig {
        // Explicit opt-out so the handler-timeout scenario is isolated
        // from signature enforcement (we want to prove the dispatch
        // path times out, not that the signature check rejected).
        WebhookConfig::default().with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned)
    }

    async fn on_activate(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl TriggerContext + ?Sized),
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

    let hanging_adapter = WebhookTriggerAdapter::new(HangingWebhook {
        meta: ActionMetadata::new(
            nebula_core::action_key!("test.webhook.hang"),
            "Hanging",
            "Handler that never returns",
        ),
    });
    let config = hanging_adapter.config().clone();
    let adapter: Arc<dyn TriggerHandler> = Arc::new(hanging_adapter);
    let ctx_template = TriggerRuntimeContext::new(
        Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(CancellationToken::new())
                .build(),
        ),
        nebula_core::WorkflowId::new(),
        nebula_core::node_key!("test"),
    );
    let handle = transport
        .activate(adapter.clone(), config, ctx_template)
        .unwrap();
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

// ── ADR-0022 trait-level coverage ────────────────────────────────────────

/// Opt-out fixture: signature policy is `OptionalAcceptUnsigned`.
/// Locks in the regression guard that tightening the default later
/// does NOT accidentally flip this webhook to 401.
struct UnsignedWebhook {
    meta: ActionMetadata,
}

impl DeclaresDependencies for UnsignedWebhook {}
impl Action for UnsignedWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for UnsignedWebhook {
    type State = ();

    fn config(&self) -> WebhookConfig {
        // Public-by-design webhook — explicit opt-out is the audit
        // trail per ADR-0022.
        WebhookConfig::default().with_signature_policy(SignaturePolicy::OptionalAcceptUnsigned)
    }

    async fn on_activate(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
    }
}

/// Fixture whose `config()` is left as the default — `Required` with
/// an empty secret. ADR-0022: the transport returns 500 without ever
/// calling `handle_request`.
struct DefaultConfigWebhook {
    meta: ActionMetadata,
    reached_handler: Arc<std::sync::atomic::AtomicBool>,
}

impl DeclaresDependencies for DefaultConfigWebhook {}
impl Action for DefaultConfigWebhook {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
}

impl WebhookAction for DefaultConfigWebhook {
    type State = ();

    // Intentionally NOT overriding config() — inherits the fail-closed
    // Required-with-empty-secret default.

    async fn on_activate(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }

    async fn handle_request(
        &self,
        _request: &WebhookRequest,
        _state: &(),
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> Result<WebhookResponse, ActionError> {
        self.reached_handler
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(WebhookResponse::accept(TriggerEventOutcome::skip()))
    }
}

async fn register_typed<A: WebhookAction>(
    transport: &WebhookTransport,
    action: A,
) -> nebula_api::services::webhook::ActivationHandle {
    let adapter = WebhookTriggerAdapter::new(action);
    let config = adapter.config().clone();
    let adapter: Arc<dyn TriggerHandler> = Arc::new(adapter);
    let ctx_template = TriggerRuntimeContext::new(
        Arc::new(
            nebula_core::BaseContext::builder()
                .cancellation(CancellationToken::new())
                .build(),
        ),
        nebula_core::WorkflowId::new(),
        nebula_core::node_key!("test"),
    );
    let handle = transport
        .activate(adapter.clone(), config, ctx_template)
        .unwrap();
    adapter.start(&handle.ctx).await.unwrap();
    handle
}

#[tokio::test]
async fn optional_accept_unsigned_passes_through_unsigned_request() {
    let transport = make_transport(None, 1024 * 1024);
    let handle = register_typed(
        &transport,
        UnsignedWebhook {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.webhook.unsigned"),
                "Unsigned",
                "OptionalAcceptUnsigned regression guard",
            ),
        },
    )
    .await;

    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    // ADR-0022: the action explicitly opted out, so the transport
    // MUST NOT block the unsigned request.
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn default_config_with_empty_secret_returns_500() {
    let transport = make_transport(None, 1024 * 1024);
    let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = register_typed(
        &transport,
        DefaultConfigWebhook {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.webhook.default_config"),
                "DefaultConfig",
                "fail-closed default enforcement",
            ),
            reached_handler: reached.clone(),
        },
    )
    .await;

    let router = transport.router();
    let request = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from("{}"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json"),
    );
    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let problem: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        problem["type"],
        "https://nebula.dev/problems/webhook-signature-misconfigured"
    );
    assert_eq!(problem["status"], 500);
    assert!(
        problem["detail"]
            .as_str()
            .is_some_and(|s| s.contains("SignaturePolicy::Required")),
    );
    // Fail-closed means the handler is NEVER reached under the default
    // empty-secret policy.
    assert!(
        !reached.load(std::sync::atomic::Ordering::Acquire),
        "handle_request must not be invoked when signature policy rejects"
    );
}

#[tokio::test]
async fn signature_failures_total_metric_increments_per_reason() {
    use nebula_metrics::{
        NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, webhook_signature_failure_reason,
    };
    use nebula_telemetry::metrics::MetricsRegistry;

    let registry = Arc::new(MetricsRegistry::new());
    let transport = WebhookTransport::with_metrics(
        WebhookTransportConfig {
            base_url: Url::parse("https://nebula.example.com").unwrap(),
            path_prefix: "/webhooks".to_string(),
            body_limit_bytes: 1024 * 1024,
            response_timeout: Duration::from_secs(5),
            rate_limit_per_minute: None,
        },
        registry.clone(),
    );

    // Fixture with a real secret so only the signature-mismatch path
    // is exercised here (the missing-secret path is covered below).
    let (handle, _) = register_webhook(&transport, b"top-secret".to_vec()).await;

    let router = transport.router();

    // Case 1: missing signature header → reason=missing.
    let req = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Case 2: present-but-bad signature → reason=invalid.
    let req = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .header("x-hub-signature-256", "sha256=deadbeef")
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let missing_labels = registry
        .interner()
        .single("reason", webhook_signature_failure_reason::MISSING);
    let invalid_labels = registry
        .interner()
        .single("reason", webhook_signature_failure_reason::INVALID);
    let missing_count = registry
        .counter_labeled(NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, &missing_labels)
        .get();
    let invalid_count = registry
        .counter_labeled(NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, &invalid_labels)
        .get();
    assert_eq!(missing_count, 1, "reason=missing must increment once");
    assert_eq!(invalid_count, 1, "reason=invalid must increment once");
}

#[tokio::test]
async fn signature_failures_total_metric_increments_missing_secret() {
    use nebula_metrics::{
        NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, webhook_signature_failure_reason,
    };
    use nebula_telemetry::metrics::MetricsRegistry;

    let registry = Arc::new(MetricsRegistry::new());
    let transport = WebhookTransport::with_metrics(
        WebhookTransportConfig {
            base_url: Url::parse("https://nebula.example.com").unwrap(),
            path_prefix: "/webhooks".to_string(),
            body_limit_bytes: 1024 * 1024,
            response_timeout: Duration::from_secs(5),
            rate_limit_per_minute: None,
        },
        registry.clone(),
    );

    let handle = register_typed(
        &transport,
        DefaultConfigWebhook {
            meta: ActionMetadata::new(
                nebula_core::action_key!("test.webhook.default_config_metric"),
                "DefaultConfigMetric",
                "missing_secret metric increment",
            ),
            reached_handler: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
    )
    .await;

    let router = transport.router();
    let req = Request::builder()
        .method("POST")
        .uri(handle.endpoint_url.path())
        .body(Body::from("{}"))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let labels = registry
        .interner()
        .single("reason", webhook_signature_failure_reason::MISSING_SECRET);
    let count = registry
        .counter_labeled(NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, &labels)
        .get();
    assert_eq!(count, 1, "reason=missing_secret must increment once");
}
