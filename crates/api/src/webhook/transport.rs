//! `WebhookTransport` — HTTP ingress layer for `nebula-action`
//! webhook triggers.
//!
//! Owns an axum `Router` mounted at
//! `POST /{path_prefix}/{trigger_uuid}/{nonce}` plus a routing map,
//! an optional rate limiter, and activation APIs for the runtime.
//!
//! # Lifecycle
//!
//! 1. Runtime starts a webhook trigger. It builds an `Arc<dyn TriggerHandler>` (a
//!    `WebhookTriggerAdapter`) and calls [`WebhookTransport::activate`], passing the handler and a
//!    template [`TriggerContext`] with all capabilities wired except `webhook` (transport fills it
//!    in).
//! 2. Transport generates a fresh `(trigger_uuid, nonce)` pair, builds an [`EndpointProviderImpl`],
//!    stores the handler + ctx in the routing map, and returns an [`ActivationHandle`].
//! 3. Runtime calls `adapter.start(&activation_handle.ctx)`.
//! 4. When HTTP requests arrive at `POST /{prefix}/{uuid}/{nonce}`, the transport looks up the
//!    entry, wraps the body in a `WebhookRequest`, attaches a oneshot response channel, dispatches
//!    to `handler.handle_event`, waits on the oneshot, and writes the response back.
//! 5. On trigger stop, runtime calls [`WebhookTransport::deactivate`] with the activation handle,
//!    which removes the entry from the routing map.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use nebula_action::{
    TriggerContext, TriggerEvent, TriggerHandler, WebhookEndpointProvider, WebhookHttpResponse,
    WebhookRequest,
};
use tokio::sync::oneshot;
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use super::{
    provider::EndpointProviderImpl,
    ratelimit::WebhookRateLimiter,
    routing::{ActivationEntry, RoutingMap},
};

/// Configuration for the webhook HTTP transport.
#[derive(Debug, Clone)]
pub struct WebhookTransportConfig {
    /// Public base URL of the Nebula API, e.g. `https://nebula.example.com`.
    /// Used to build the full URL handed to webhook actions via
    /// `ctx.webhook.endpoint_url()`.
    pub base_url: Url,
    /// Path prefix under which webhook routes are mounted, e.g. `/webhooks`.
    pub path_prefix: String,
    /// Maximum body size accepted from external callers, in bytes.
    /// Anything larger returns `413 Payload Too Large`.
    pub body_limit_bytes: usize,
    /// How long to wait on the oneshot response channel after
    /// dispatching to `handle_event` before returning
    /// `504 Gateway Timeout`.
    pub response_timeout: Duration,
    /// Per-path requests-per-minute cap. `None` disables per-path
    /// rate limiting entirely.
    pub rate_limit_per_minute: Option<u64>,
}

impl Default for WebhookTransportConfig {
    fn default() -> Self {
        Self {
            // Safe-ish fallback. Production deployments MUST override
            // this via config.
            base_url: Url::parse("http://localhost:8080").expect("static URL"),
            path_prefix: "/webhooks".to_string(),
            body_limit_bytes: 1024 * 1024, // 1 MiB matches nebula-action default
            response_timeout: Duration::from_secs(10),
            rate_limit_per_minute: None,
        }
    }
}

/// Returned by [`WebhookTransport::activate`]. Hold onto this for
/// the lifetime of the trigger registration; pass it back to
/// [`WebhookTransport::deactivate`] on stop.
#[derive(Debug)]
pub struct ActivationHandle {
    trigger_uuid: Uuid,
    nonce: String,
    /// Per-activation context template populated with the webhook
    /// endpoint capability. Runtime clones this into the trigger's
    /// `start()` call.
    pub ctx: TriggerContext,
    /// Fully-resolved URL the action hands to the external provider
    /// in `on_activate`. Same value is exposed inside `ctx.webhook`.
    pub endpoint_url: Url,
}

/// HTTP ingress layer for webhook triggers.
#[derive(Clone)]
pub struct WebhookTransport {
    inner: Arc<TransportInner>,
}

struct TransportInner {
    config: WebhookTransportConfig,
    routing: RoutingMap,
    rate_limiter: Option<WebhookRateLimiter>,
}

impl std::fmt::Debug for WebhookTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookTransport")
            .field("path_prefix", &self.inner.config.path_prefix)
            .field("base_url", &self.inner.config.base_url.as_str())
            .finish_non_exhaustive()
    }
}

impl WebhookTransport {
    /// Build a new transport from config.
    #[must_use]
    pub fn new(config: WebhookTransportConfig) -> Self {
        let rate_limiter = config.rate_limit_per_minute.map(WebhookRateLimiter::new);
        Self {
            inner: Arc::new(TransportInner {
                config,
                routing: RoutingMap::new(),
                rate_limiter,
            }),
        }
    }

    /// Register a webhook trigger and allocate its public endpoint.
    ///
    /// Builds a fresh `(uuid, nonce)` pair, constructs an
    /// [`EndpointProviderImpl`], injects it into the supplied
    /// `ctx_template` via
    /// [`TriggerContext::with_webhook_endpoint`](nebula_action::TriggerContext::with_webhook_endpoint),
    /// and stores the pair in the routing map. Caller takes the
    /// returned [`ActivationHandle`] and passes `handle.ctx` to
    /// `adapter.start(...)`.
    pub fn activate(
        &self,
        handler: Arc<dyn TriggerHandler>,
        ctx_template: TriggerContext,
    ) -> Result<ActivationHandle, ActivationError> {
        let trigger_uuid = Uuid::new_v4();
        let nonce = generate_nonce();
        let provider = EndpointProviderImpl::new(
            &self.inner.config.base_url,
            &self.inner.config.path_prefix,
            trigger_uuid,
            &nonce,
        )
        .map_err(ActivationError::InvalidBaseUrl)?;
        let endpoint_url = provider.endpoint_url().clone();
        let ctx = ctx_template.with_webhook_endpoint(Arc::new(provider));

        let entry = ActivationEntry {
            handler,
            ctx: ctx.clone(),
        };
        if !self
            .inner
            .routing
            .insert(trigger_uuid, nonce.clone(), entry)
        {
            // Should be unreachable with a freshly-generated nonce.
            return Err(ActivationError::DuplicateRegistration);
        }

        Ok(ActivationHandle {
            trigger_uuid,
            nonce,
            ctx,
            endpoint_url,
        })
    }

    /// Remove a previously-activated registration from the routing
    /// map. Idempotent — safe to call twice.
    pub fn deactivate(&self, handle: &ActivationHandle) {
        self.inner
            .routing
            .remove(&handle.trigger_uuid, &handle.nonce);
    }

    /// Build the axum router that dispatches incoming webhook
    /// requests to registered triggers.
    ///
    /// The route is gated to `POST` only: the webhook contract at
    /// the top of this file specifies `POST /{prefix}/{trigger_uuid}/{nonce}`,
    /// and axum returns `405 Method Not Allowed` with `Allow: POST`
    /// automatically for non-POST requests. Gating at the routing
    /// boundary (rather than inside the handler) means middlewares,
    /// rate limiters, and the routing-map lookup never count the
    /// offending request — cheaper *and* more correct.
    pub fn router(&self) -> Router {
        let route = format!(
            "{prefix}/{{trigger_uuid}}/{{nonce}}",
            prefix = self.inner.config.path_prefix,
        );
        Router::new()
            .route(&route, post(webhook_handler))
            .layer(DefaultBodyLimit::max(self.inner.config.body_limit_bytes))
            .with_state(self.clone())
    }
}

/// Errors returned by [`WebhookTransport::activate`].
#[derive(Debug, thiserror::Error)]
pub enum ActivationError {
    /// `base_url` in config cannot be combined with the computed
    /// path — usually means `base_url` is not origin-only.
    #[error("invalid webhook base_url: {0}")]
    InvalidBaseUrl(#[source] url::ParseError),
    /// Routing map already held an entry for the generated
    /// `(uuid, nonce)`. Effectively unreachable because the nonce is
    /// freshly generated; this variant exists so the activate path
    /// does not silently swallow a collision bug.
    #[error("duplicate webhook registration (nonce collision)")]
    DuplicateRegistration,
}

/// Generate a 32-character random hex nonce.
///
/// 128 bits of entropy — enough to make nonce collisions
/// impossible over the lifetime of any Nebula deployment. Uses
/// `Uuid::new_v4` under the hood because uuid is already pulled in.
fn generate_nonce() -> String {
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();
    let mut out = String::with_capacity(32);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ── HTTP handler ────────────────────────────────────────────────────────

/// Axum handler for `POST /{prefix}/{trigger_uuid}/{nonce}`.
///
/// Error-to-status mapping follows the spec:
///
/// | Situation                            | Status |
/// |--------------------------------------|--------|
/// | Unknown `(uuid, nonce)`              | 404    |
/// | Invalid UUID in path                 | 404    |
/// | Body exceeds `body_limit_bytes`      | 413    |
/// | Header count exceeds 256             | 400    |
/// | Rate limit exceeded                  | 429    |
/// | Handler returns `ActionError` (any)  | 500    |
/// | Oneshot timeout                      | 504    |
/// | Oneshot RecvError (unexpected)       | 500    |
async fn webhook_handler(
    State(transport): State<WebhookTransport>,
    Path((trigger_uuid_str, nonce)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // 1. Parse UUID — malformed path segment → 404.
    let trigger_uuid = match Uuid::parse_str(&trigger_uuid_str) {
        Ok(u) => u,
        Err(_) => return (StatusCode::NOT_FOUND, "").into_response(),
    };

    // 2. Body size check. Axum's `Bytes` extractor consumes the
    // entire body; we enforce the cap AFTER extraction because
    // `axum::extract::DefaultBodyLimit` is applied at the router
    // level and we want a domain-specific 413 with our cap number.
    if body.len() > transport.inner.config.body_limit_bytes {
        debug!(
            size = body.len(),
            cap = transport.inner.config.body_limit_bytes,
            "webhook body exceeds cap"
        );
        return (StatusCode::PAYLOAD_TOO_LARGE, "").into_response();
    }

    // 3. Rate limit (if configured).
    if let Some(limiter) = &transport.inner.rate_limiter {
        let key = format!("{trigger_uuid}/{nonce}");
        if let Err(e) = limiter.check(&key).await {
            debug!(path = %key, retry_after = e.retry_after_secs, "webhook rate limited");
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, "").into_response();
            if let Ok(v) = e.retry_after_secs.to_string().parse() {
                resp.headers_mut().insert("retry-after", v);
            }
            return resp;
        }
    }

    // 4. Route lookup.
    let entry = match transport.inner.routing.lookup(&trigger_uuid, &nonce) {
        Some(e) => e,
        None => {
            debug!(uuid = %trigger_uuid, "no webhook registered for path");
            return (StatusCode::NOT_FOUND, "").into_response();
        },
    };

    // 5. Construct WebhookRequest. Limits are already enforced by
    // `try_new` — the only failures here are body-size exceed
    // (handled above for a better error message) and header count
    // exceed (rare; returns 400).
    let path = uri.path().to_string();
    let query = uri.query().map(String::from);
    let request = match WebhookRequest::try_new(method, path, query, headers, body) {
        Ok(r) => r,
        Err(nebula_action::ActionError::DataLimitExceeded { .. }) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "").into_response();
        },
        Err(e) => {
            debug!(error = %e, "webhook request construction failed");
            return (StatusCode::BAD_REQUEST, "").into_response();
        },
    };

    // 6. Oneshot response channel.
    let (tx, rx) = oneshot::channel::<WebhookHttpResponse>();
    let request = request.with_response_channel(tx);
    let event = TriggerEvent::new(None, request);

    // 7. Dispatch with timeout. The handler sends the HTTP response
    // through the oneshot; we race that against the configured
    // `response_timeout`.
    let handler = Arc::clone(&entry.handler);
    let ctx = entry.ctx.clone();
    let timeout = transport.inner.config.response_timeout;
    let dispatch_fut = async move { handler.handle_event(event, &ctx).await };

    let dispatch_result = tokio::time::timeout(timeout, dispatch_fut).await;

    match dispatch_result {
        Ok(Ok(_outcome)) => {
            // Outcome is the workflow-emission outcome; it's already
            // been used by the adapter to record health. The HTTP
            // response comes through the oneshot the adapter sent to
            // right before returning Ok.
            match rx.await {
                Ok(http) => http_response_to_axum(http),
                Err(_) => {
                    warn!("webhook handler returned Ok but oneshot sender was dropped");
                    (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
                },
            }
        },
        Ok(Err(e)) => {
            // Handler returned an error. The adapter (after H1 fix)
            // ALREADY sent a 500 via the oneshot before returning
            // Err. We just read it.
            debug!(error = %e, "webhook handler returned error");
            match rx.await {
                Ok(http) => http_response_to_axum(http),
                Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
            }
        },
        Err(_) => {
            warn!(
                timeout_secs = timeout.as_secs(),
                "webhook handler dispatch timed out"
            );
            (StatusCode::GATEWAY_TIMEOUT, "").into_response()
        },
    }
}

/// Convert a `nebula-action` `WebhookHttpResponse` into an axum
/// `Response`. Shared between the Ok and Err dispatch paths.
fn http_response_to_axum(resp: WebhookHttpResponse) -> Response {
    (resp.status, resp.headers, resp.body).into_response()
}
