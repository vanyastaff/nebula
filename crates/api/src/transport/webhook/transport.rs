//! `WebhookTransport` — HTTP ingress layer for `nebula-action`
//! webhook triggers.
//!
//! Owns an axum `Router` mounted at
//! `POST /{path_prefix}/{trigger_uuid}/{nonce}` plus a routing map,
//! an optional rate limiter, and activation APIs for the runtime.
//!
//! The router is merged into [`crate::build_app`], so **W3C Trace Context**
//! (`traceparent` / `tracestate`) on requests and responses matches the public API stack
//! (M3.5 — extraction, `TraceLayer` parent linking, and response injection).
//!
//! # Lifecycle
//!
//! 1. Runtime starts a webhook trigger. It builds an `Arc<dyn TriggerHandler>` (a
//!    `WebhookTriggerAdapter`) and calls [`WebhookTransport::activate`], passing the handler and a
//!    template `TriggerContext` with all capabilities wired except `webhook` (transport fills it
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
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::post,
};
use nebula_action::{
    Clock, SignatureError, SignatureOutcome, SignaturePolicy, SystemClock, TriggerEvent,
    TriggerHandler, TriggerRuntimeContext, WebhookConfig, WebhookEndpointProvider,
    WebhookHttpResponse, WebhookRequest,
};
use nebula_metrics::MetricsRegistry;
use nebula_metrics::{
    NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL,
    NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, webhook_key_kind, webhook_replay_rejection_reason,
    webhook_signature_failure_reason,
};
use tokio::sync::oneshot;
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use super::ratelimit::WebhookRateLimiter;
use super::{
    key::WebhookKey,
    provider::EndpointProviderImpl,
    routing::{ActivationEntry, RoutingMap},
};
use crate::error::ProblemDetails;

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
    pub ctx: TriggerRuntimeContext,
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
    /// Optional metrics registry. When `Some`, signature-failure
    /// outcomes increment [`NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`]
    /// per ADR-0022. `None` means the transport runs without emitting
    /// the counter — the enforcement behaviour is identical.
    metrics: Option<Arc<MetricsRegistry>>,
    /// Time source for replay-window enforcement (M3.3 / ADR-0049).
    /// Production deployments use [`SystemClock`]; tests inject
    /// `MockClock` to drive deterministic timestamp scenarios.
    clock: Arc<dyn Clock>,
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
    /// Build a new transport from config. Defaults to [`SystemClock`].
    #[must_use]
    pub fn new(config: WebhookTransportConfig) -> Self {
        Self::build(config, None, Arc::new(SystemClock::new()))
    }

    /// Build a new transport from config with a metrics registry
    /// attached. Signature-failure outcomes increment
    /// [`NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`] per ADR-0022.
    ///
    /// Composition roots that already own a `MetricsRegistry` (the
    /// API crate's `AppState` does) should prefer this constructor
    /// over [`Self::new`]. Without a registry the transport still
    /// enforces the signature policy; it just does not emit the
    /// per-failure counter.
    #[must_use]
    pub fn with_metrics(config: WebhookTransportConfig, metrics: Arc<MetricsRegistry>) -> Self {
        Self::build(config, Some(metrics), Arc::new(SystemClock::new()))
    }

    /// Build a transport with metrics and a custom [`Clock`]. Tests
    /// pass `Arc::new(MockClock::at_unix_secs(...))` to drive
    /// deterministic replay-window scenarios.
    #[must_use]
    pub fn with_metrics_and_clock(
        config: WebhookTransportConfig,
        metrics: Arc<MetricsRegistry>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::build(config, Some(metrics), clock)
    }

    fn build(
        config: WebhookTransportConfig,
        metrics: Option<Arc<MetricsRegistry>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let rate_limiter = config.rate_limit_per_minute.map(WebhookRateLimiter::new);
        Self {
            inner: Arc::new(TransportInner {
                config,
                routing: RoutingMap::new(),
                rate_limiter,
                metrics,
                clock,
            }),
        }
    }

    /// Register a webhook trigger and allocate its public endpoint.
    ///
    /// Builds a fresh `(uuid, nonce)` pair, constructs an
    /// [`EndpointProviderImpl`], injects it into the supplied
    /// `ctx_template` via
    /// `TriggerContext::with_webhook_endpoint` (in the `nebula_action` crate),
    /// and stores the pair in the routing map. Caller takes the
    /// returned [`ActivationHandle`] and passes `handle.ctx` to
    /// `adapter.start(...)`.
    ///
    /// `action_config` is the [`WebhookConfig`] read from the typed
    /// [`nebula_action::WebhookAction`] that the handler wraps. Per
    /// ADR-0022 the caller reads it from the action (typically via
    /// `WebhookTriggerAdapter::config()`) before erasing the handler
    /// to `Arc<dyn TriggerHandler>` — webhook-specific configuration
    /// does not flow through the dyn trigger contract.
    pub fn activate(
        &self,
        handler: Arc<dyn TriggerHandler>,
        action_config: WebhookConfig,
        ctx_template: TriggerRuntimeContext,
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
            config: action_config,
        };
        let key = WebhookKey::programmatic(trigger_uuid, nonce.clone());
        if !self.inner.routing.insert(key, entry) {
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
        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce.clone());
        self.inner.routing.remove(&key);
    }

    /// Register a slug-routed activation (M3.3 / ADR-0049).
    ///
    /// The handler comes from a [`nebula_action::WebhookActionFactory`]
    /// invoked by the API bootstrap (E1) or the lifecycle subscriber
    /// (E2). `config` is the [`WebhookConfig`] cached on the wrapping
    /// adapter; the bootstrap reads it from
    /// [`nebula_action::BuiltWebhookHandler::config`].
    ///
    /// `ctx` is a per-activation [`TriggerRuntimeContext`] template
    /// the transport clones on every dispatch — same discipline as
    /// programmatic [`Self::activate`].
    ///
    /// # Errors
    ///
    /// Returns [`ActivationError::DuplicateRegistration`] if a slug
    /// activation already exists at `coords`. Pair with
    /// [`Self::unregister_slug`] for explicit replacement, or use
    /// [`Self::replace_slug_map`] for atomic bulk swaps.
    pub fn activate_slug(
        &self,
        coords: super::key::TriggerCoordinates,
        handler: Arc<dyn TriggerHandler>,
        config: WebhookConfig,
        ctx: TriggerRuntimeContext,
    ) -> Result<(), ActivationError> {
        let entry = ActivationEntry {
            handler,
            ctx,
            config,
        };
        let key = WebhookKey::slug(coords);
        if !self.inner.routing.insert(key, entry) {
            return Err(ActivationError::DuplicateRegistration);
        }
        Ok(())
    }

    /// Remove a slug activation. Idempotent.
    pub fn unregister_slug(&self, coords: &super::key::TriggerCoordinates) -> bool {
        let key = WebhookKey::Slug(coords.clone());
        self.inner.routing.remove(&key)
    }

    /// Number of slug-routed activations currently in the routing
    /// map. Driven by E1 bootstrap, E2 lifecycle subscriber, and E3
    /// admin reload.
    #[must_use]
    pub fn slug_count(&self) -> usize {
        self.inner.routing.count_by_kind("slug")
    }

    /// Total active registrations (programmatic + slug). Used by
    /// `/healthz` reporters.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.inner.routing.len()
    }

    /// Atomic swap of all slug activations — used by the admin reload
    /// endpoint (E3) so external observers do not see a half-loaded
    /// routing table during a multi-thousand-row reload. Programmatic
    /// activations are preserved.
    pub fn replace_slug_map(
        &self,
        new: Vec<(
            super::key::TriggerCoordinates,
            Arc<dyn TriggerHandler>,
            WebhookConfig,
            TriggerRuntimeContext,
        )>,
    ) {
        let payload = new
            .into_iter()
            .map(|(coords, handler, config, ctx)| {
                (
                    WebhookKey::Slug(coords),
                    ActivationEntry {
                        handler,
                        ctx,
                        config,
                    },
                )
            })
            .collect();
        self.inner.routing.replace_slug_entries(payload);
    }

    /// Build the axum router that dispatches incoming webhook
    /// requests to registered triggers.
    ///
    /// Mounts both URL shapes (M3.3 / ADR-0049):
    ///
    /// - Programmatic: `POST {path_prefix}/{trigger_uuid}/{nonce}` —
    ///   minted by [`Self::activate`].
    /// - Slug: `POST /api/v1/hooks/{org}/{ws}/{slug}` and
    ///   `GET /api/v1/hooks/{org}/{ws}/{slug}` (provider-specific
    ///   challenge handshakes via `pre_handle`) — registered by
    ///   [`Self::activate_slug`].
    ///
    /// Both routes funnel into `dispatch_inner` for a single
    /// source of truth on signature, replay, rate-limit, and
    /// pre-handle pipelines.
    pub fn router(&self) -> Router {
        let programmatic = format!(
            "{prefix}/{{trigger_uuid}}/{{nonce}}",
            prefix = self.inner.config.path_prefix,
        );
        Router::new()
            .route(&programmatic, post(webhook_handler))
            .route(
                "/api/v1/hooks/{org}/{ws}/{trigger_slug}",
                post(slug_webhook_handler).get(slug_webhook_handler),
            )
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
    let key = WebhookKey::programmatic(trigger_uuid, nonce);
    dispatch_inner(transport, key, method, uri, headers, body).await
}

/// Axum handler for `POST /api/v1/hooks/{org}/{ws}/{slug}`. Builds a
/// [`WebhookKey::Slug`] and delegates to `dispatch_inner` — same
/// signature/replay/rate-limit/pre-handle/handle pipeline as the
/// programmatic surface.
async fn slug_webhook_handler(
    State(transport): State<WebhookTransport>,
    Path((org, workspace, trigger)): Path<(String, String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let coords = super::key::TriggerCoordinates::new(org, workspace, trigger);
    let key = WebhookKey::Slug(coords);
    dispatch_inner(transport, key, method, uri, headers, body).await
}

/// Shared dispatch pipeline for both programmatic and slug webhook
/// surfaces (M3.3 / ADR-0049). Order of operations:
///
/// 1. body size check → 413
/// 2. rate-limit by [`WebhookKey`] → 429 + `Retry-After`
/// 3. routing lookup → 404
/// 4. construct [`WebhookRequest`] → 400 / 413
/// 5. [`enforce_signature`] (uses [`Clock`]) → 401 / 500
/// 6. dispatch via [`TriggerHandler::handle_event`] (the adapter
///    runs `pre_handle` internally before `handle_request`) with a
///    response timeout → 504 / 500 / handler response
async fn dispatch_inner(
    transport: WebhookTransport,
    key: WebhookKey,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Body size check. Axum's `Bytes` extractor consumes the entire
    // body; we enforce the cap AFTER extraction because
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

    // 3. Route lookup BEFORE rate-limit so attacker churn through
    // unregistered keys cannot evict legitimate buckets from the
    // LRU-bounded path table (#271 follow-up). Unregistered keys
    // never touch the limiter.
    let entry = if let Some(e) = transport.inner.routing.lookup(&key) {
        e
    } else {
        debug!(key = ?key, "no webhook registered for path");
        return (StatusCode::NOT_FOUND, "").into_response();
    };

    // 4. Rate limit (if configured) — only for keys that resolve to a
    // registered handler.
    if let Some(limiter) = &transport.inner.rate_limiter {
        let bucket = key.rate_limit_key();
        if let Err(e) = limiter.check(&bucket).await {
            debug!(path = %bucket, retry_after = e.retry_after_secs, "webhook rate limited");
            record_rate_limit_rejection(&transport, &key);
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, "").into_response();
            if let Ok(v) = e.retry_after_secs.to_string().parse() {
                resp.headers_mut().insert("retry-after", v);
            }
            return resp;
        }
    }

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

    // 5.5. Signature enforcement (ADR-0022). The `Required` default
    // means an action that forgot to configure a secret trips a 500
    // here; an action that explicitly opted into
    // `OptionalAcceptUnsigned` passes through; everything else
    // (hex / base64 / custom) runs through the existing constant-time
    // primitives before the handler sees the request.
    match enforce_signature(
        entry.config.signature_policy(),
        &request,
        transport.inner.clock.as_ref(),
    ) {
        SignatureVerdict::Pass => {},
        SignatureVerdict::MissingSecret => {
            record_signature_failure(&transport, webhook_signature_failure_reason::MISSING_SECRET);
            return missing_secret_response(uri.path());
        },
        SignatureVerdict::Fail(reason) => {
            record_signature_failure(&transport, reason);
            return signature_rejected_response(uri.path(), reason);
        },
    }

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
            if let Ok(http) = rx.await {
                http_response_to_axum(http)
            } else {
                warn!("webhook handler returned Ok but oneshot sender was dropped");
                (StatusCode::INTERNAL_SERVER_ERROR, "").into_response()
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

// ── Signature enforcement (ADR-0022) ────────────────────────────────────────

/// Result of transport-layer signature enforcement.
///
/// Internal type — the handler turns it into a 2xx pass-through or a
/// problem+json 4xx/5xx response.
enum SignatureVerdict {
    /// Either the policy is `OptionalAcceptUnsigned` or the configured
    /// verifier returned `Valid`. Request dispatch continues.
    Pass,
    /// `Required` policy held an empty secret — no signature can be
    /// verified. Returns 500 (our misconfiguration, not the caller's).
    MissingSecret,
    /// Signature header absent (`missing`) or present-but-mismatched
    /// (`invalid`). Returns 401.
    Fail(&'static str),
}

/// Run the configured signature check against the request.
///
/// Any non-`Valid` outcome from a `Required` or `Custom` policy is a
/// 401; only an empty secret under `Required` is a 500. An `ActionError`
/// surfaced by the primitives (bad header name, empty secret before
/// the empty-secret check) is treated as `Invalid` — a 401 — rather
/// than a 500, because the payload came from an external caller and
/// the specific error is not the operator's to debug.
fn enforce_signature(
    policy: &SignaturePolicy,
    request: &WebhookRequest,
    clock: &dyn Clock,
) -> SignatureVerdict {
    match policy {
        SignaturePolicy::OptionalAcceptUnsigned => SignatureVerdict::Pass,
        SignaturePolicy::Required(req) => match req.verify_with(request, clock) {
            Ok(()) => SignatureVerdict::Pass,
            Err(SignatureError::SecretMissing) => SignatureVerdict::MissingSecret,
            Err(SignatureError::SignatureMissing) => {
                SignatureVerdict::Fail(webhook_signature_failure_reason::MISSING)
            },
            Err(SignatureError::SignatureInvalid) => {
                SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID)
            },
            Err(SignatureError::TimestampMissing) => {
                SignatureVerdict::Fail(webhook_signature_failure_reason::TIMESTAMP_MISSING)
            },
            Err(SignatureError::TimestampMalformed { reason }) => {
                debug!(reason = %reason, "webhook timestamp malformed");
                SignatureVerdict::Fail(webhook_signature_failure_reason::TIMESTAMP_MALFORMED)
            },
            Err(SignatureError::TimestampOutOfWindow { skew_secs }) => {
                debug!(skew_secs, "webhook timestamp outside replay window");
                SignatureVerdict::Fail(webhook_signature_failure_reason::TIMESTAMP_OUT_OF_WINDOW)
            },
            // `SignatureError` is `#[non_exhaustive]` — fail-closed
            // on any future variant.
            Err(_) => SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID),
        },
        SignaturePolicy::Custom(verifier) => outcome_to_verdict(verifier(request)),
    }
}

fn outcome_to_verdict(outcome: SignatureOutcome) -> SignatureVerdict {
    match outcome {
        SignatureOutcome::Valid => SignatureVerdict::Pass,
        SignatureOutcome::Missing => {
            SignatureVerdict::Fail(webhook_signature_failure_reason::MISSING)
        },
        SignatureOutcome::Invalid => {
            SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID)
        },
        // `SignatureOutcome` is `#[non_exhaustive]`; any future variant
        // is fail-closed.
        _ => SignatureVerdict::Fail(webhook_signature_failure_reason::INVALID),
    }
}

fn record_signature_failure(transport: &WebhookTransport, reason: &'static str) {
    if let Some(reg) = &transport.inner.metrics {
        let labels = reg.interner().single("reason", reason);
        if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL, &labels) {
            c.inc();
        }
        // Replay-window failures double-bump the dedicated replay
        // counter so dashboards can isolate them from generic
        // signature mismatches without scraping the `reason` label.
        // All three timestamp-related signature failure modes count
        // as replay-window enforcement.
        if let Some(replay_reason) = replay_reason_for(reason) {
            let labels = reg.interner().single("reason", replay_reason);
            if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_REPLAY_REJECTIONS_TOTAL, &labels) {
                c.inc();
            }
        }
    }
}

/// Map a signature-failure reason to a replay-window rejection
/// reason when the failure is timestamp-related. Returns `None` for
/// failure modes unrelated to replay enforcement (missing signature,
/// signature invalid, missing secret).
fn replay_reason_for(reason: &str) -> Option<&'static str> {
    match reason {
        r if r == webhook_signature_failure_reason::TIMESTAMP_OUT_OF_WINDOW => {
            Some(webhook_replay_rejection_reason::TIMESTAMP_OUT_OF_WINDOW)
        },
        r if r == webhook_signature_failure_reason::TIMESTAMP_MISSING => {
            Some(webhook_replay_rejection_reason::TIMESTAMP_MISSING)
        },
        r if r == webhook_signature_failure_reason::TIMESTAMP_MALFORMED => {
            Some(webhook_replay_rejection_reason::TIMESTAMP_MALFORMED)
        },
        _ => None,
    }
}

/// Record a per-key rate-limit rejection. Labelset:
/// `(tenant_id, webhook_key_kind)`.
fn record_rate_limit_rejection(transport: &WebhookTransport, key: &WebhookKey) {
    let Some(reg) = &transport.inner.metrics else {
        return;
    };
    let interner = reg.interner();
    let kind = match key {
        WebhookKey::Programmatic { .. } => webhook_key_kind::PROGRAMMATIC,
        WebhookKey::Slug(_) => webhook_key_kind::SLUG,
    };
    let tenant_id = match key {
        WebhookKey::Programmatic { .. } => "programmatic".to_owned(),
        WebhookKey::Slug(coords) => format!("{}/{}", coords.org, coords.workspace),
    };
    let labels = interner.label_set(&[
        ("webhook_key_kind", kind),
        ("tenant_id", tenant_id.as_str()),
    ]);
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_RATE_LIMIT_REJECTIONS_TOTAL, &labels) {
        c.inc();
    }
}

/// 401 response for a signature mismatch. RFC 9457 `application/problem+json`
/// to match the rest of the API surface.
fn signature_rejected_response(instance_path: &str, reason: &'static str) -> Response {
    let detail = match reason {
        r if r == webhook_signature_failure_reason::MISSING => {
            "webhook signature header missing".to_string()
        },
        r if r == webhook_signature_failure_reason::INVALID => {
            "webhook signature invalid".to_string()
        },
        other => format!("webhook signature rejected: {other}"),
    };
    let problem = ProblemDetails::new(
        "https://nebula.dev/problems/webhook-signature",
        "Webhook Signature Rejected",
        StatusCode::UNAUTHORIZED,
    )
    .with_detail(detail)
    .with_instance(instance_path.to_string());
    problem_response(StatusCode::UNAUTHORIZED, problem)
}

/// 500 response for an action that shipped `Required` without a secret.
/// This is fail-closed behaviour — the author's misconfiguration
/// surfaces as a server error so it shows up in dashboards rather than
/// silently accepting unsigned requests.
fn missing_secret_response(instance_path: &str) -> Response {
    let problem = ProblemDetails::new(
        "https://nebula.dev/problems/webhook-signature-misconfigured",
        "Webhook Signature Secret Not Configured",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .with_detail(
        "webhook action declared SignaturePolicy::Required but no HMAC secret is configured; \
         supply one via WebhookConfig::with_signature_policy + \
         SignaturePolicy::required_hmac_sha256, or explicitly opt out with \
         SignaturePolicy::OptionalAcceptUnsigned"
            .to_string(),
    )
    .with_instance(instance_path.to_string());
    problem_response(StatusCode::INTERNAL_SERVER_ERROR, problem)
}

/// Static `application/problem+json` content-type header value.
///
/// Constant-constructed at compile time via [`HeaderValue::from_static`] —
/// no runtime `.parse().expect(...)` panic path. RFC 9457 normalizes the
/// exact token, so this is the canonical value for every problem+json
/// response the transport emits.
const PROBLEM_JSON_CONTENT_TYPE: HeaderValue = HeaderValue::from_static("application/problem+json");

/// Shared assembly for `application/problem+json` responses returned
/// from the webhook transport. Matches the convention in
/// [`crate::error::ApiError::into_response`].
fn problem_response(status: StatusCode, problem: ProblemDetails) -> Response {
    let mut resp = (status, Json(problem)).into_response();
    resp.headers_mut()
        .insert(axum::http::header::CONTENT_TYPE, PROBLEM_JSON_CONTENT_TYPE);
    resp
}
