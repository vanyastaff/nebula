//! Webhook trigger domain — DX trait, adapter, HMAC signature primitives,
//! and the canonical HTTP-shaped event type ([`WebhookRequest`]).
//!
//! Groups everything an action author needs to implement a webhook
//! trigger into one file:
//!
//! - [`WebhookRequest`] — typed HTTP event (method, path, query, headers, body, arrival timestamp).
//! Constructed by the HTTP transport layer via [`WebhookRequest::try_new`] /
//! [`WebhookRequest::try_new_with_limits`] which enforce body-size and header-count limits at the
//! boundary.
//! - [`WebhookAction`] — DX trait with the register / handle / unregister lifecycle. Implement this
//! and register via `registry.register_webhook(action)`.
//! - [`WebhookTriggerAdapter`] — bridges a typed `WebhookAction` to the
//! [`TriggerHandler`](crate::trigger::TriggerHandler) dyn contract. Stores state from
//! `on_activate` in a `RwLock<Option<Arc<State>>>`, rejects double-start with
//! `ActionError::Fatal`, cleans up orphaned state on lost-race rollback, and downcasts incoming
//! [`TriggerEvent`](crate::trigger::TriggerEvent)s to [`WebhookRequest`].
//! - [`verify_hmac_sha256`], [`hmac_sha256_compute`], [`verify_tag_constant_time`],
//! [`SignatureOutcome`] — constant-time HMAC primitives. Use `verify_hmac_sha256` for
//! GitHub-style `sha256=…` bare-hex signatures; reach for the lower-level pair for Stripe / Slack
//! schemes that sign a derived payload.
//! - [`WebhookConfig`], [`SignaturePolicy`], [`RequiredPolicy`], [`SignatureScheme`] — the
//! `Required`-by-default signature-enforcement contract at the trait surface. Authors return a
//! [`WebhookConfig`] from [`WebhookAction::config`]; the HTTP transport reads its
//! [`WebhookConfig::signature_policy`] before dispatch; misconfigured or unsigned requests are
//! rejected before the action sees them. See.
//!
//! # Security
//!
//! **Never compare signatures with `==` or `str::eq`.** The byte-wise
//! short-circuit in `PartialEq` leaks the secret one prefix byte at a
//! time and is exploitable over the network. Always use the helpers in
//! this module.
//!
//! **Signature verification is `Required` by default.** An action that
//! does not override [`WebhookAction::config`] inherits an empty-secret
//! `Required` policy — the transport returns `500 problem+json` with a
//! "webhook signature secret not configured" diagnostic until a secret
//! is supplied. Authors who want unsigned webhooks must explicitly
//! return a [`WebhookConfig`] carrying
//! [`SignaturePolicy::OptionalAcceptUnsigned`]; the override itself is
//! the audit trail.
//!
//! Stripe/Slack helpers are intentionally NOT provided: their correct
//! implementation requires a time source and a tolerance window to
//! prevent replay, and wrapping that correctly would pull platform
//! clocks into this module. Build them in your action on top of the
//! primitives.

mod clock;
pub mod factory;
pub mod providers;
mod source;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, SystemTime},
};

pub use clock::{Clock, MockClock, SystemClock};
pub use factory::{BuiltWebhookHandler, FactoryError, WebhookActionFactory, WebhookActivationSpec};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use bytes::Bytes;
use hmac::{Hmac, KeyInit, Mac};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use parking_lot::RwLock;
use sha2::Sha256;
pub use source::WebhookSource;
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, oneshot};
use tracing::{debug, warn};

use crate::{
    action::Action,
    context::TriggerContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    trigger::{TriggerEvent, TriggerEventOutcome, TriggerHandler},
};

// ── Limits ──────────────────────────────────────────────────────────────────

/// Default maximum webhook body size accepted by [`WebhookRequest::try_new`]: 1 MiB.
///
/// Rationale: covers 99% of real-world webhook payloads. Reference providers:
/// - Stripe: 256 KB per event
/// - Slack: ~3 MB
/// - GitHub push events: typically < 100 KB (release artifacts can hit 25 MB)
/// - Twilio: 64 KB
///
/// For providers that need more, use [`WebhookRequest::try_new_with_limits`]
/// and pass an explicit cap — we want oversize intake to be a deliberate
/// decision visible in code review, not an accident.
pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Maximum header count accepted by [`WebhookRequest::try_new`].
///
/// RFC 9110 does not mandate a limit; most HTTP servers cap at 100.
/// We allow 256 to accommodate modern deployments that stack
/// Cloudflare + NGINX + a service-mesh sidecar in front of the
/// webhook endpoint — each layer typically adds 10–30 tracing,
/// forwarding, and auth headers, so 60–80 is a legitimate baseline
/// and 128 (our previous cap) was tight. 256 still prevents an
/// O(n²) CPU DoS if an action calls `header()` in a loop against a
/// huge attacker-supplied header set.
pub const MAX_HEADER_COUNT: usize = 256;

// ── WebhookRequest ──────────────────────────────────────────────────────────

/// Incoming HTTP webhook request — the typed event carried inside a
/// [`TriggerEvent`] for webhook triggers.
///
/// The HTTP transport layer constructs one of these from an incoming
/// request and wraps it in a `TriggerEvent` before routing to the
/// trigger adapter. Action authors receive `&WebhookRequest` inside
/// [`WebhookAction::handle_request`].
///
/// # Fields are private
///
/// Access is via accessors so the adapter can enforce two invariants:
///
/// 1. **Body bytes are canonical for HMAC verification.** The body is immutable once constructed —
///    action authors cannot rewrite it between signature check and payload parsing.
/// 2. **Size and header limits are checked at construction.** The only way to build a
///    `WebhookRequest` is through [`try_new`](Self::try_new) /
///    [`try_new_with_limits`](Self::try_new_with_limits), so no caller can bypass the limits by
///    constructing one manually.
pub struct WebhookRequest {
    method: Method,
    path: String,
    query: Option<String>,
    headers: HeaderMap,
    body: Bytes,
    received_at: SystemTime,
    response_tx: parking_lot::Mutex<Option<oneshot::Sender<WebhookHttpResponse>>>,
}

impl WebhookRequest {
    /// Build a request with the default caps ([`DEFAULT_MAX_BODY_BYTES`] /
    /// [`MAX_HEADER_COUNT`]).
    ///
    /// `method` / `path` / `query` / `headers` / `body` are the raw
    /// pieces of the incoming HTTP request. The arrival timestamp is
    /// set to the current clock; use
    /// [`with_received_at`](Self::with_received_at) to override when
    /// replaying persisted events.
    ///
    /// For providers whose payloads exceed the defaults (e.g., GitHub
    /// release events up to 25 MB), use
    /// [`try_new_with_limits`](Self::try_new_with_limits).
    ///
    /// # Errors
    ///
    /// - [`ActionError::DataLimitExceeded`] if `body.len() > DEFAULT_MAX_BODY_BYTES`
    /// - [`ActionError::Validation`] if `headers.len() > MAX_HEADER_COUNT`
    pub fn try_new(
        method: Method,
        path: impl Into<String>,
        query: Option<impl Into<String>>,
        headers: HeaderMap,
        body: impl Into<Bytes>,
    ) -> Result<Self, ActionError> {
        Self::try_new_with_limits(
            method,
            path,
            query,
            headers,
            body,
            DEFAULT_MAX_BODY_BYTES,
            MAX_HEADER_COUNT,
        )
    }

    /// Build a request with explicit limits. Prefer [`try_new`](Self::try_new)
    /// unless you know your provider exceeds the defaults.
    ///
    /// # Errors
    ///
    /// - [`ActionError::DataLimitExceeded`] if `body.len() > max_body`
    /// - [`ActionError::Validation`] if header count exceeds `max_headers`
    pub fn try_new_with_limits(
        method: Method,
        path: impl Into<String>,
        query: Option<impl Into<String>>,
        headers: HeaderMap,
        body: impl Into<Bytes>,
        max_body: usize,
        max_headers: usize,
    ) -> Result<Self, ActionError> {
        let body: Bytes = body.into();

        if body.len() > max_body {
            return Err(ActionError::DataLimitExceeded {
                limit_bytes: max_body as u64,
                actual_bytes: body.len() as u64,
            });
        }

        // `HeaderMap::len` counts distinct header values, not unique names;
        // close enough for a DoS cap that wants "no more than N entries".
        if headers.len() > max_headers {
            return Err(ActionError::validation(
                "headers",
                ValidationReason::OutOfRange,
                Some(format!(
                    "too many headers on webhook request: {} > {max_headers}",
                    headers.len()
                )),
            ));
        }

        Ok(Self {
            method,
            path: path.into(),
            query: query.map(Into::into),
            headers,
            body,
            received_at: SystemTime::now(),
            response_tx: parking_lot::Mutex::new(None),
        })
    }

    /// Override the arrival timestamp. Useful for replay from persisted
    /// events where the original arrival time should be preserved.
    #[must_use]
    pub fn with_received_at(mut self, at: SystemTime) -> Self {
        self.received_at = at;
        self
    }

    /// HTTP method (`GET`, `POST`, etc.).
    ///
    /// Slack's URL verification handshake arrives as `POST` but some
    /// providers use `GET` for challenge exchanges — authors that
    /// support both must branch on this.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// URL path (without query string). Useful for multi-tenant routing
    /// where one webhook endpoint serves several tenants by path prefix.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Raw query string (without the leading `?`), if any.
    ///
    /// Callers that need structured access should parse this with
    /// `url::form_urlencoded` or `serde_urlencoded`.
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// Full header map. Use [`header`](Self::header) for a single typed lookup.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Look up a header value as a string slice (case-insensitive via
    /// `HeaderMap`). Returns `None` if the header is missing or if the
    /// value contains non-ASCII bytes.
    ///
    /// Multi-valued headers return the first value; use
    /// [`headers`](Self::headers) for full access.
    #[must_use]
    pub fn header(&self, name: &HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// Look up a header value by string name. Convenience over
    /// [`header`](Self::header) for call sites that already know the
    /// header as a `&str` and don't want to build a `HeaderName`.
    ///
    /// Returns `None` if the name is not a valid HTTP header name,
    /// the header is missing, or the value is not valid ASCII.
    #[must_use]
    pub fn header_str(&self, name: &str) -> Option<&str> {
        let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        self.header(&name)
    }

    /// Raw body bytes — canonical form for HMAC signature verification.
    ///
    /// Do NOT round-trip through [`body_json`](Self::body_json) before
    /// computing a signature: serde's output is not byte-identical to
    /// the original body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Body as a UTF-8 string slice, if it is valid UTF-8.
    #[must_use]
    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Parse the body as JSON.
    ///
    /// The body is already bounded by the construction-time limit, so
    /// this call inherits that byte-size DoS protection.
    ///
    /// # Security — depth not checked
    ///
    /// `serde_json` does **not** cap recursion depth by default. A
    /// hostile 1 MiB body containing 100 000 levels of `{"a":`
    /// followed by matching `}` will happily parse and may overflow
    /// the tokio worker thread stack (2 MiB on Linux release
    /// builds). For webhook endpoints that parse untrusted JSON,
    /// prefer [`body_json_bounded`](Self::body_json_bounded) which
    /// pre-scans the byte stream for nesting depth before invoking
    /// `serde_json`.
    ///
    /// Use `body_json` only when you trust the payload source (e.g.,
    /// a provider that has its own schema enforcement) or when your
    /// target type deserialises into a small fixed-depth structure.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the body is not valid JSON.
    pub fn body_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Parse the body as JSON with a hard cap on nesting depth.
    ///
    /// Scans the body bytes once, tracking `{`/`[` vs `}`/`]` depth
    /// and skipping the contents of JSON strings (with `\\`/`\"`
    /// escape handling). If the depth at any point exceeds
    /// `max_depth`, returns a custom serde error WITHOUT invoking
    /// `serde_json`.
    ///
    /// Recommended `max_depth`: **64**. Real webhook payloads rarely
    /// exceed 10 levels; 64 leaves headroom for deeply nested
    /// metadata (Slack blocks, GitHub check runs) while staying well
    /// below stack-overflow territory.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the body is not valid JSON, or if the
    /// pre-scan reports nesting depth above `max_depth`.
    pub fn body_json_bounded<T: serde::de::DeserializeOwned>(
        &self,
        max_depth: usize,
    ) -> Result<T, serde_json::Error> {
        check_json_depth(&self.body, max_depth)?;
        serde_json::from_slice(&self.body)
    }

    /// Arrival timestamp at the transport. Used for replay-window
    /// validation (reject events whose `received_at` is older than
    /// the tolerance window) and for runtime metrics.
    #[must_use]
    pub fn received_at(&self) -> SystemTime {
        self.received_at
    }

    /// Stable per-delivery id for transport-level idempotency.
    ///
    /// Checks common webhook delivery-id headers in precedence order:
    /// - `X-Delivery-ID` (general convention)
    /// - `X-GitHub-Delivery` (GitHub)
    ///
    /// Returns the first non-empty ASCII-valid header value found, or `None`
    /// if no known delivery-id header is present.
    ///
    /// Use this from [`crate::TriggerAction::idempotency_key`] to surface
    /// a transport-supplied dedup id to the engine without re-parsing headers.
    #[must_use]
    pub fn delivery_id(&self) -> Option<&str> {
        // Check headers in precedence order. Uses `header_str` for
        // case-insensitive lookup and ASCII validation. Empty or
        // whitespace-only header values are treated as absent so a
        // blank `X-Delivery-ID` does not shadow a populated fallback
        // header and never produces an empty `IdempotencyKey`.
        self.header_str("x-delivery-id")
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .or_else(|| {
                self.header_str("x-github-delivery")
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
            })
    }

    /// Attach a response channel for HTTP response plumbing.
    ///
    /// The HTTP transport layer calls this before wrapping the request
    /// in a [`TriggerEvent`]. The adapter sends a [`WebhookHttpResponse`]
    /// through the channel after calling [`WebhookAction::handle_request`].
    #[must_use]
    pub fn with_response_channel(self, tx: oneshot::Sender<WebhookHttpResponse>) -> Self {
        *self.response_tx.lock() = Some(tx);
        self
    }

    /// Take the response channel sender, if present.
    ///
    /// Called by adapters after downcast. The sender is moved out so
    /// that `handle_request` sees an immutable `&WebhookRequest`.
    pub(crate) fn take_response_tx(&self) -> Option<oneshot::Sender<WebhookHttpResponse>> {
        self.response_tx.lock().take()
    }
}

impl fmt::Debug for WebhookRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("query", &self.query)
            .field("headers_len", &self.headers.len())
            .field("body_len", &self.body.len())
            .field("received_at", &self.received_at)
            .field("has_response_channel", &self.response_tx.lock().is_some())
            .finish()
    }
}

// ── HTTP response types ────────────────────────────────────────────────────

/// HTTP response data sent back to the webhook caller.
///
/// Produced by [`WebhookResponse::into_parts`] and plumbed through the
/// response channel attached to [`WebhookRequest::with_response_channel`].
/// The transport layer receives this and writes it to the HTTP connection.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WebhookHttpResponse {
    /// HTTP status code.
    pub status: StatusCode,
    /// Response headers.
    pub headers: HeaderMap,
    /// Response body.
    pub body: Bytes,
}

impl WebhookHttpResponse {
    /// 200 OK with empty body — the default for [`WebhookResponse::Accept`].
    #[must_use]
    pub fn ok() -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::new(),
        }
    }

    /// Custom status with body and empty headers.
    #[must_use]
    pub fn new(status: StatusCode, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.into(),
        }
    }

    /// Add response headers (builder pattern).
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }
}

impl Default for WebhookHttpResponse {
    fn default() -> Self {
        Self::ok()
    }
}

/// Return type from [`WebhookAction::handle_request`].
///
/// Controls both the HTTP response sent to the webhook caller and the
/// workflow emission triggered by the event. The two concerns are
/// orthogonal: a Slack URL verification handshake returns a challenge
/// body (`Respond`) but does not emit a workflow (`Skip`), while a
/// normal push event returns 200 OK (`Accept`) and emits.
///
/// # Examples
///
/// ```rust,ignore
/// // Normal event — accept with default 200 OK, emit workflow.
/// Ok(WebhookResponse::accept(TriggerEventOutcome::emit(payload)))
///
/// // Slack URL verification — echo challenge, no workflow.
/// Ok(WebhookResponse::respond(
/// StatusCode::OK,
/// challenge_value,
/// TriggerEventOutcome::skip(),
/// ))
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum WebhookResponse {
    /// Accept with 200 OK (default HTTP response).
    Accept(TriggerEventOutcome),
    /// Send a custom HTTP response.
    Respond {
        /// HTTP response to send back to the caller.
        http: WebhookHttpResponse,
        /// Workflow emission outcome.
        outcome: TriggerEventOutcome,
    },
}

impl WebhookResponse {
    /// Accept with default 200 OK.
    #[must_use]
    pub fn accept(outcome: TriggerEventOutcome) -> Self {
        Self::Accept(outcome)
    }

    /// Custom HTTP response with empty headers.
    #[must_use]
    pub fn respond(
        status: StatusCode,
        body: impl Into<Bytes>,
        outcome: TriggerEventOutcome,
    ) -> Self {
        Self::Respond {
            http: WebhookHttpResponse::new(status, body),
            outcome,
        }
    }

    /// Custom HTTP response with headers.
    #[must_use]
    pub fn respond_with_headers(
        status: StatusCode,
        headers: HeaderMap,
        body: impl Into<Bytes>,
        outcome: TriggerEventOutcome,
    ) -> Self {
        Self::Respond {
            http: WebhookHttpResponse::new(status, body).with_headers(headers),
            outcome,
        }
    }

    /// Reference to the workflow emission outcome.
    #[must_use]
    pub fn outcome(&self) -> &TriggerEventOutcome {
        match self {
            Self::Accept(o) | Self::Respond { outcome: o, .. } => o,
        }
    }

    /// Split into HTTP response data and workflow emission outcome.
    #[must_use]
    pub fn into_parts(self) -> (WebhookHttpResponse, TriggerEventOutcome) {
        match self {
            Self::Accept(outcome) => (WebhookHttpResponse::ok(), outcome),
            Self::Respond { http, outcome } => (http, outcome),
        }
    }
}

// ── DX trait ────────────────────────────────────────────────────────────────

/// Webhook trigger — register / handle / unregister lifecycle.
///
/// Implement `handle_request` (required), and optionally `on_activate`
/// / `on_deactivate`. Register via `registry.register_webhook(action)`.
///
/// State from `on_activate` is stored by the adapter and passed to
/// `handle_request` (by reference) and `on_deactivate` (by value). For
/// mutable per-event state, wrap fields in `Mutex` or atomic types
/// inside `Self::State`.
///
/// # Example
///
/// Use [`verify_hmac_sha256`] for constant-time signature verification —
/// naive `==` comparison on HMAC digests leaks the secret via a
/// prefix-length timing side-channel.
///
/// ```rust,ignore
/// use nebula_action::webhook::{WebhookAction, WebhookRequest, WebhookResponse, verify_hmac_sha256};
/// use nebula_action::trigger::TriggerEventOutcome;
///
/// struct GitHubWebhook { secret: Vec<u8> }
///
/// impl WebhookAction for GitHubWebhook {
/// type State = WebhookReg;
///
/// async fn on_activate(&self, ctx: &(impl TriggerContext + ?Sized)) -> Result<WebhookReg, ActionError> {
/// Ok(WebhookReg { hook_id: register(ctx).await? })
/// }
///
/// async fn handle_request(&self, req: &WebhookRequest, _state: &Self::State, _ctx: &(impl TriggerContext + ?Sized))
/// -> Result<WebhookResponse, ActionError> {
/// let outcome = verify_hmac_sha256(req, &self.secret, "X-Hub-Signature-256")?;
/// if !outcome.is_valid() {
/// return Ok(WebhookResponse::accept(TriggerEventOutcome::skip()));
/// }
/// Ok(WebhookResponse::accept(TriggerEventOutcome::emit(req.body_json()?)))
/// }
///
/// async fn on_deactivate(&self, state: WebhookReg, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
/// delete_hook(&state.hook_id).await
/// }
/// }
/// ```
pub trait WebhookAction: Action + Send + Sync + 'static {
    /// State held between activate/deactivate (e.g., webhook registration ID).
    ///
    /// No serialization or `Default` bounds — the action creates state in
    /// [`on_activate`](Self::on_activate) and receives it back in
    /// [`on_deactivate`](Self::on_deactivate). If the runtime needs to
    /// persist state across restarts, that is the runtime's
    /// responsibility (post-v1).
    type State: Clone + Send + Sync;

    /// Register webhook with external service. Returns state to persist.
    ///
    /// **Required** — no default implementation.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if registration fails.
    fn on_activate(
        &self,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send;

    /// Handle an incoming webhook request.
    ///
    /// Return [`WebhookResponse::accept`] to send 200 OK, or
    /// [`WebhookResponse::respond`] for a custom HTTP response (e.g.,
    /// Slack URL verification challenge echo). The
    /// [`TriggerEventOutcome`] inside the response controls workflow
    /// emission independently of the HTTP response.
    ///
    /// State from `on_activate` is passed by reference. The adapter
    /// clones an internal `Arc` cheaply before this call — no
    /// contention with start/stop.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if event processing fails.
    fn handle_request(
        &self,
        request: &WebhookRequest,
        state: &Self::State,
        ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<WebhookResponse, ActionError>> + Send;

    /// Inspect a request **after** signature + replay verification
    /// but **before** [`Self::handle_request`].
    ///
    /// Provider-typed actions (Slack, Stripe, Generic) override this
    /// to intercept verification probes — Slack's `url_verification`
    /// POST, Stripe's `pending_webhook` ping, generic
    /// `?challenge=<token>` GETs. Returning
    /// [`PreHandleOutcome::RespondNow`] hands the response straight
    /// back to the caller without touching the engine.
    ///
    /// # Order of operations in the transport
    ///
    /// `signature.verify_with` → `pre_handle` → `handle_request`. Probes
    /// must therefore present a valid signature when the provider
    /// requires one (this is the case for Slack and Stripe — both sign
    /// their verification probes the same way as normal events).
    ///
    /// # Default
    ///
    /// `Ok(PreHandleOutcome::Continue)` — every request goes through
    /// to `handle_request`. Existing implementors compile unchanged.
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] only for hard internal failures. Bad
    /// requests should be answered with `RespondNow(...)` carrying a
    /// 4xx body rather than an `Err`.
    fn pre_handle(
        &self,
        _request: &WebhookRequest,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<PreHandleOutcome, ActionError>> + Send {
        async { Ok(PreHandleOutcome::Continue) }
    }

    /// Unregister webhook on deactivation.
    ///
    /// Receives the state stored from `on_activate`. Default: no-op.
    /// Not called if `on_activate` was never called (stop without start).
    ///
    /// # Errors
    ///
    /// Return [`ActionError`] if unregistration fails.
    fn on_deactivate(
        &self,
        _state: Self::State,
        _ctx: &(impl TriggerContext + ?Sized),
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    /// Webhook-specific configuration the transport layer enforces
    /// around dispatch: signature policy today; future slots for
    /// body-limit / rate-limit overrides are reserved on
    /// [`WebhookConfig`] via `#[non_exhaustive]` so they can land
    /// without a trait-method churn.
    ///
    /// Default: [`WebhookConfig::default`] — signature policy is
    /// `Required` with an empty secret, meaning the transport returns
    /// `500 problem+json` until an author supplies a secret via
    /// [`WebhookConfig::with_signature_policy`] /
    /// [`SignaturePolicy::required_hmac_sha256`]. Fail-closed is the
    /// whole point; silently accepting unsigned POSTs is the bug this
    /// method exists to prevent.
    ///
    /// Authors opt into unsigned-acceptable webhooks by setting
    /// [`SignaturePolicy::OptionalAcceptUnsigned`] with a doc-comment
    /// justification — the override in source *is* the audit trail.
    /// Providers whose signature scheme is not hex / base64 (Stripe,
    /// Slack) use [`SignaturePolicy::Custom`] with a verifier that
    /// composes the primitives in this module.
    ///
    ///
    fn config(&self) -> WebhookConfig {
        WebhookConfig::default()
    }
}

// ── WebhookConfig ────────────────────────────────────────────────────────────

/// Webhook-specific transport configuration returned by
/// [`WebhookAction::config`].
///
/// Opaque bag-type so new webhook-layer settings (body-limit override,
/// per-trigger rate-limit override, untrusted-depth guard) can land in
/// future ADRs without changing the trait signature. `#[non_exhaustive]`
/// commits that discipline at the type level.
///
/// # Fields
///
/// - [`signature_policy`](Self::signature_policy). Default:
///   `Required` with an empty secret (fail-closed).
///
/// # Cloning
///
/// `WebhookConfig` is `Clone` — it holds `SignaturePolicy`, which is
/// itself cheap to clone (`Arc<[u8]>` for the required-policy secret,
/// `Arc<dyn Fn>` for the custom verifier).
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct WebhookConfig {
    signature_policy: SignaturePolicy,
    /// Optional provider tag. Set by provider-typed actions
    /// ([`WebhookProvider::Slack`], [`WebhookProvider::Stripe`],
    /// [`WebhookProvider::Generic`]); `None` for ad-hoc actions.
    /// The transport reads this to decide telemetry labels and to
    /// gate provider-specific lifecycle hooks.
    provider: Option<WebhookProvider>,
}

impl WebhookConfig {
    /// Default config: `Required` signature policy with an empty
    /// secret (fail-closed). Equivalent to [`Self::default`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the signature policy.
    #[must_use]
    pub fn with_signature_policy(mut self, policy: SignaturePolicy) -> Self {
        self.signature_policy = policy;
        self
    }

    /// Tag this config with a provider. Required for actions that
    /// rely on provider-specific [`WebhookAction::pre_handle`]
    /// behaviour (Slack `url_verification`, Stripe `pending_webhook`,
    /// Generic challenge endpoints).
    #[must_use]
    pub fn with_provider(mut self, provider: WebhookProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Signature policy enforced by the transport before dispatch.
    #[must_use]
    pub fn signature_policy(&self) -> &SignaturePolicy {
        &self.signature_policy
    }

    /// Provider tag, if set via [`Self::with_provider`].
    #[must_use]
    pub fn provider(&self) -> Option<&WebhookProvider> {
        self.provider.as_ref()
    }
}

// ── WebhookProvider ──────────────────────────────────────────────────────

/// Provider catalog tag for a [`WebhookAction`].
///
/// Lets the transport (and metrics layer) tell apart provider-specific
/// flows without inspecting [`std::any::TypeId`] or reflecting on the
/// underlying action type. New providers extend this enum without
/// breaking external consumers thanks to `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WebhookProvider {
    /// Slack — verification via `url_verification` POSTs and
    /// `X-Slack-Signature` HMAC of `v0:{ts}:{body}`.
    Slack,
    /// Stripe — verification via `Stripe-Signature: t=…,v1=…` and
    /// the `pending_webhook` ping payload during endpoint creation.
    Stripe,
    /// Generic provider-agnostic webhook. May expose a configurable
    /// `?challenge=<token>` GET endpoint mirroring Microsoft-style
    /// validation (Teams, SharePoint).
    Generic {
        /// Optional shared token compared in constant time against the
        /// inbound `?challenge=` query parameter. `None` disables the
        /// GET challenge endpoint.
        challenge_token: Option<String>,
    },
}

// ── PreHandleOutcome ─────────────────────────────────────────────────────

/// Result of [`WebhookAction::pre_handle`].
///
/// `Continue` — request flows to [`WebhookAction::handle_request`].
/// `RespondNow(resp)` — transport returns `resp` verbatim and
/// **skips** `handle_request` entirely. Use this for protocol-level
/// challenges (Slack `url_verification`, Stripe `pending_webhook`
/// ping, generic provider GET challenge) where the response is the
/// whole point of the request and no engine work should be done.
#[derive(Debug)]
#[non_exhaustive]
pub enum PreHandleOutcome {
    /// Default: dispatch continues to `handle_request`.
    Continue,
    /// Short-circuit: return this HTTP response and skip
    /// `handle_request`.
    RespondNow(WebhookHttpResponse),
}

// ── SignaturePolicy ──────────────────────────────────────────────────────────

/// Signature-verification policy for a [`WebhookAction`].
///
/// Carried on [`WebhookConfig`] and read by the HTTP transport
/// before dispatching an incoming request to the action — a
/// misconfigured or unsigned request never reaches
/// [`WebhookAction::handle_request`].
///
/// # Default is fail-closed
///
/// [`SignaturePolicy::default`] is [`Self::Required`] with an empty
/// secret. The transport returns `500 problem+json` until an author
/// supplies a secret. Authors who want to accept unsigned requests
/// must explicitly return [`Self::OptionalAcceptUnsigned`]; the
/// override is the audit trail.
///
/// # Cloning
///
/// `SignaturePolicy` is `Clone` — `Required` holds `Arc<[u8]>` and
/// `HeaderName` (both cheap clones) and `Custom` holds `Arc<dyn Fn>`.
/// The adapter reads the policy once at construction and stores it.
///
///
pub enum SignaturePolicy {
    /// Require HMAC signature via the configured header and scheme.
    /// Default: `X-Nebula-Signature`, hex-encoded SHA-256, empty secret.
    Required(RequiredPolicy),
    /// Explicit opt-out. Transport accepts any request, signed or not.
    /// Use only for public-by-design webhooks (Slack URL verification,
    /// open repo webhooks) or local testing.
    OptionalAcceptUnsigned,
    /// Escape hatch for signature schemes the standard verifiers do
    /// not cover — Stripe's `t=…,v1=…` with a replay window, Shopify
    /// base64 with a derived payload, bespoke provider conventions.
    /// Compose via [`verify_hmac_sha256_with_timestamp`],
    /// [`hmac_sha256_compute`], and [`verify_tag_constant_time`].
    Custom(Arc<dyn Fn(&WebhookRequest) -> SignatureOutcome + Send + Sync>),
}

impl Default for SignaturePolicy {
    fn default() -> Self {
        Self::Required(RequiredPolicy::default())
    }
}

impl Clone for SignaturePolicy {
    fn clone(&self) -> Self {
        match self {
            Self::Required(r) => Self::Required(r.clone()),
            Self::OptionalAcceptUnsigned => Self::OptionalAcceptUnsigned,
            Self::Custom(f) => Self::Custom(Arc::clone(f)),
        }
    }
}

impl fmt::Debug for SignaturePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Required(r) => f.debug_tuple("SignaturePolicy::Required").field(r).finish(),
            Self::OptionalAcceptUnsigned => f.write_str("SignaturePolicy::OptionalAcceptUnsigned"),
            Self::Custom(_) => f.write_str("SignaturePolicy::Custom(..)"),
        }
    }
}

impl SignaturePolicy {
    /// Construct a [`Self::Required`] policy populated with the given
    /// HMAC secret. Header defaults to `X-Nebula-Signature`, scheme to
    /// [`SignatureScheme::Sha256Hex`].
    #[must_use]
    pub fn required_hmac_sha256(secret: impl Into<Arc<[u8]>>) -> Self {
        Self::Required(RequiredPolicy::new().with_secret(secret))
    }

    /// Construct a [`Self::Custom`] policy wrapping the provided
    /// verifier closure. The closure receives the raw request and
    /// returns a [`SignatureOutcome`]; the transport treats anything
    /// other than `Valid` as a 401.
    #[must_use]
    pub fn custom<F>(verifier: F) -> Self
    where
        F: Fn(&WebhookRequest) -> SignatureOutcome + Send + Sync + 'static,
    {
        Self::Custom(Arc::new(verifier))
    }
}

/// Configuration for [`SignaturePolicy::Required`].
///
/// Builder-style: start from [`RequiredPolicy::new`] or
/// [`RequiredPolicy::default`], then chain [`with_secret`](Self::with_secret),
/// [`with_header`](Self::with_header), [`with_scheme`](Self::with_scheme) as needed.
///
/// # Empty secret
///
/// An empty secret is *not* a misuse of the API — it is the
/// fail-closed default surface that catches "author forgot to
/// supply a secret" at the transport layer. See.
///
/// # Non-exhaustive
///
/// Marked `#[non_exhaustive]` so future webhook-layer settings
/// (replay window, timestamp header) can land without breaking
/// external constructors. Build via [`RequiredPolicy::new`] or
/// [`RequiredPolicy::default`] and chain `with_*` setters.
#[derive(Clone)]
#[non_exhaustive]
pub struct RequiredPolicy {
    secret: Arc<[u8]>,
    header: HeaderName,
    scheme: SignatureScheme,
    /// Header carrying the request timestamp (Stripe / Slack style).
    /// `None` skips replay-window enforcement, preserving behaviour
    /// for actions that have not opted in.
    timestamp_header: Option<HeaderName>,
    /// Encoding of the timestamp header value. Default
    /// [`TimestampFormat::UnixSeconds`].
    timestamp_format: TimestampFormat,
    /// Maximum acceptable skew between the request timestamp and
    /// `clock.now()`. Default 300 s (5 min). Future timestamps beyond
    /// 60 s are also rejected as a hard cap.
    replay_window: Duration,
}

/// Default replay window. Matches the Slack and Stripe defaults; OK
/// upstream of provider-specific overrides.
const DEFAULT_REPLAY_WINDOW: Duration = Duration::from_mins(5);

/// Hard ceiling on positive forward skew (timestamp dated *after*
/// `clock.now()`). A replay window that accepts arbitrary future
/// timestamps is no replay window at all — pin the future side
/// independently of the configured window.
const FUTURE_SKEW_SECS: i64 = 60;

impl RequiredPolicy {
    /// Canonical Nebula default: `X-Nebula-Signature`,
    /// [`SignatureScheme::Sha256Hex`], empty secret (fail-closed until
    /// an author supplies one), no timestamp header (replay-window
    /// enforcement opt-in).
    #[must_use]
    pub fn new() -> Self {
        Self {
            secret: Arc::from(Vec::<u8>::new()),
            header: HeaderName::from_static("x-nebula-signature"),
            scheme: SignatureScheme::Sha256Hex,
            timestamp_header: None,
            timestamp_format: TimestampFormat::UnixSeconds,
            replay_window: DEFAULT_REPLAY_WINDOW,
        }
    }

    /// Replace the secret. An empty secret keeps the policy in the
    /// fail-closed state and the transport returns 500.
    #[must_use]
    pub fn with_secret(mut self, secret: impl Into<Arc<[u8]>>) -> Self {
        self.secret = secret.into();
        self
    }

    /// Replace the signature header name. Providers typically need
    /// their own (`X-Hub-Signature-256` for GitHub,
    /// `X-Shopify-Hmac-SHA256` for Shopify).
    ///
    /// # Errors
    ///
    /// Returns `Err` if `header` is not a valid HTTP header name.
    pub fn with_header_str(self, header: &str) -> Result<Self, ActionError> {
        let header = HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
            ActionError::validation(
                "webhook.signature_policy.header",
                ValidationReason::WrongType,
                Some(format!("invalid HTTP header name: {header:?}")),
            )
        })?;
        Ok(self.with_header(header))
    }

    /// Replace the signature header with a pre-validated [`HeaderName`].
    #[must_use]
    pub fn with_header(mut self, header: HeaderName) -> Self {
        self.header = header;
        self
    }

    /// Replace the signature scheme.
    #[must_use]
    pub fn with_scheme(mut self, scheme: SignatureScheme) -> Self {
        self.scheme = scheme;
        self
    }

    /// Opt into replay-window enforcement by naming the timestamp
    /// header. Pair with [`Self::with_replay_window`] and
    /// [`Self::with_timestamp_format`] when the provider differs from
    /// the defaults (Unix seconds, 5-minute window).
    #[must_use]
    pub fn with_timestamp_header(mut self, header: HeaderName) -> Self {
        self.timestamp_header = Some(header);
        self
    }

    /// String-based variant of [`Self::with_timestamp_header`].
    ///
    /// # Errors
    ///
    /// Returns `Err` if `header` is not a valid HTTP header name.
    pub fn with_timestamp_header_str(self, header: &str) -> Result<Self, ActionError> {
        let name = HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
            ActionError::validation(
                "webhook.signature_policy.timestamp_header",
                ValidationReason::WrongType,
                Some(format!("invalid HTTP header name: {header:?}")),
            )
        })?;
        Ok(self.with_timestamp_header(name))
    }

    /// Replace the timestamp encoding. See [`TimestampFormat`].
    #[must_use]
    pub fn with_timestamp_format(mut self, format: TimestampFormat) -> Self {
        self.timestamp_format = format;
        self
    }

    /// Replace the replay window. Stripe/Slack default is 300 s;
    /// providers with looser tolerances should configure their own.
    #[must_use]
    pub fn with_replay_window(mut self, window: Duration) -> Self {
        self.replay_window = window;
        self
    }

    /// Shared HMAC secret. Empty slice indicates the fail-closed
    /// default; transports treat that identically to a missing
    /// credential.
    #[must_use]
    pub fn secret(&self) -> &[u8] {
        &self.secret
    }

    /// Header carrying the signature value.
    #[must_use]
    pub fn header(&self) -> &HeaderName {
        &self.header
    }

    /// Signature encoding scheme.
    #[must_use]
    pub fn scheme(&self) -> SignatureScheme {
        self.scheme
    }

    /// Timestamp header name. `None` means replay-window enforcement
    /// is disabled for this policy.
    #[must_use]
    pub fn timestamp_header(&self) -> Option<&HeaderName> {
        self.timestamp_header.as_ref()
    }

    /// Timestamp encoding (Unix seconds, Unix milliseconds, or RFC
    /// 3339).
    #[must_use]
    pub fn timestamp_format(&self) -> TimestampFormat {
        self.timestamp_format
    }

    /// Replay window — the maximum skew between the request
    /// timestamp and the clock.
    #[must_use]
    pub fn replay_window(&self) -> Duration {
        self.replay_window
    }

    /// Run the configured timestamp + signature checks against `request`.
    ///
    /// The single source of truth shared by the HTTP transport's
    /// programmatic and slug-routed paths. Timestamp validation runs
    /// **before** HMAC math so cheap rejections do not pay the
    /// constant-time signature cost.
    ///
    /// # Order of operations
    ///
    /// 1. Empty secret → [`SignatureError::SecretMissing`] (500 at the transport).
    /// 2. If `timestamp_header` is set: pull, parse, and replay-check.
    /// 3. HMAC verification via the configured [`SignatureScheme`].
    ///
    /// When `timestamp_header` is `None`, step 2 is skipped — preserves
    /// behaviour for actions that have not opted into replay
    /// protection.
    ///
    /// # Errors
    ///
    /// Returns the first failing step as a typed [`SignatureError`].
    pub fn verify_with(
        &self,
        request: &WebhookRequest,
        clock: &dyn Clock,
    ) -> Result<(), SignatureError> {
        if self.secret.is_empty() {
            return Err(SignatureError::SecretMissing);
        }

        if let Some(name) = self.timestamp_header.as_ref() {
            validate_timestamp(
                request.headers(),
                name,
                self.timestamp_format,
                self.replay_window,
                clock,
            )?;
        }

        let outcome = match self.scheme {
            SignatureScheme::Sha256Hex => {
                match verify_hmac_sha256(request, &self.secret, self.header.as_str()) {
                    Ok(o) => o,
                    Err(error) => {
                        warn!(
                            scheme = "sha256-hex",
                            %error,
                            "webhook signature primitive returned unexpected error; treating as Invalid"
                        );
                        SignatureOutcome::Invalid
                    },
                }
            },
            SignatureScheme::Sha256Base64 => {
                match verify_hmac_sha256_base64(request, &self.secret, self.header.as_str()) {
                    Ok(o) => o,
                    Err(error) => {
                        warn!(
                            scheme = "sha256-base64",
                            %error,
                            "webhook signature primitive returned unexpected error; treating as Invalid"
                        );
                        SignatureOutcome::Invalid
                    },
                }
            },
            SignatureScheme::StandardWebhooks => {
                verify_standard_webhooks(request, &self.secret, self.replay_window, clock)?
            },
        };

        match outcome {
            SignatureOutcome::Valid => Ok(()),
            SignatureOutcome::Missing => Err(SignatureError::SignatureMissing),
            SignatureOutcome::Invalid => Err(SignatureError::SignatureInvalid),
        }
    }
}

impl Default for RequiredPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RequiredPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequiredPolicy")
            .field(
                "secret",
                &format_args!("<redacted len={}>", self.secret.len()),
            )
            .field("header", &self.header)
            .field("scheme", &self.scheme)
            .field("timestamp_header", &self.timestamp_header)
            .field("timestamp_format", &self.timestamp_format)
            .field("replay_window", &self.replay_window)
            .finish()
    }
}

// ── Timestamp / replay-window enforcement ───────────────────────────────────

/// Encoding of a timestamp header value (Stripe / Slack / RFC 3339).
///
/// `RequiredPolicy::timestamp_format` selects which decoder
/// `validate_timestamp` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TimestampFormat {
    /// Stripe's `t=…`, Slack's `X-Slack-Request-Timestamp`. The
    /// canonical default.
    #[default]
    UnixSeconds,
    /// Some bespoke providers ship millisecond-resolution Unix epochs.
    UnixMillis,
    /// RFC 3339 / ISO 8601 timestamps. Acceptable forms: `2026-05-07T14:23:00Z`,
    /// `2026-05-07T14:23:00+00:00`. Sub-second precision is preserved.
    Rfc3339,
}

/// Failure modes for [`RequiredPolicy::verify_with`].
///
/// Each variant carries enough context for the transport layer to
/// emit RFC 9457 problem+json without re-deriving the cause. The
/// transport maps:
/// - `SecretMissing` → 500 (operator misconfig)
/// - `SignatureMissing` / `SignatureInvalid` → 401
/// - `TimestampMissing` / `TimestampMalformed` / `TimestampOutOfWindow` → 401
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SignatureError {
    /// `Required` policy held an empty secret. 500 — operator
    /// misconfiguration, not a caller fault.
    #[error("webhook signature secret not configured")]
    SecretMissing,
    /// Signature header absent. 401.
    #[error("webhook signature header missing")]
    SignatureMissing,
    /// Signature header present but did not match. 401.
    #[error("webhook signature invalid")]
    SignatureInvalid,
    /// Timestamp header configured but absent on request. 401.
    #[error("webhook timestamp header missing")]
    TimestampMissing,
    /// Timestamp header present but unparsable. 401.
    #[error("webhook timestamp malformed: {reason}")]
    TimestampMalformed {
        /// One-line cause; safe to echo into problem+json detail.
        reason: String,
    },
    /// Timestamp parsed but outside the replay window. 401.
    #[error("webhook timestamp out of replay window: skew {skew_secs}s")]
    TimestampOutOfWindow {
        /// Positive when the request timestamp is older than `now`,
        /// negative when it is in the future. Useful for diagnosing
        /// clock skew on the producer side.
        skew_secs: i64,
    },
}

/// Decode and replay-check a single timestamp header value.
///
/// Pure: takes the raw [`HeaderMap`], the configured header name, the
/// expected encoding, the replay window, and a [`Clock`]. Never
/// touches the wall clock directly. Future timestamps beyond a
/// hard-coded 60-second forward-skew cap (private constant
/// `FUTURE_SKEW_SECS`) are rejected even if the configured window
/// would technically admit them — a replay window that accepts
/// arbitrary future timestamps is no replay window at all.
pub fn validate_timestamp(
    headers: &HeaderMap,
    header_name: &HeaderName,
    format: TimestampFormat,
    window: Duration,
    clock: &dyn Clock,
) -> Result<(), SignatureError> {
    let raw = match single_header_value(headers, header_name) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Err(SignatureError::TimestampMissing),
        HeaderLookup::Multiple => {
            return Err(SignatureError::TimestampMalformed {
                reason: "multiple timestamp headers".to_string(),
            });
        },
    };

    let ts_secs = parse_timestamp_secs(raw, format)?;
    let now_secs: i64 = match clock.now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => {
            return Err(SignatureError::TimestampMalformed {
                reason: "clock predates Unix epoch".to_string(),
            });
        },
    };

    let skew = now_secs.saturating_sub(ts_secs);
    let window_secs = window.as_secs() as i64;
    if skew > window_secs || skew < -FUTURE_SKEW_SECS {
        return Err(SignatureError::TimestampOutOfWindow { skew_secs: skew });
    }

    debug!(
        skew_secs = skew,
        header_name = %header_name,
        "webhook timestamp accepted"
    );
    Ok(())
}

fn parse_timestamp_secs(raw: &str, format: TimestampFormat) -> Result<i64, SignatureError> {
    let trimmed = raw.trim();
    match format {
        TimestampFormat::UnixSeconds => {
            trimmed
                .parse::<i64>()
                .map_err(|_| SignatureError::TimestampMalformed {
                    reason: format!("not a Unix-seconds integer: {trimmed:?}"),
                })
        },
        TimestampFormat::UnixMillis => {
            let millis =
                trimmed
                    .parse::<i64>()
                    .map_err(|_| SignatureError::TimestampMalformed {
                        reason: format!("not a Unix-milliseconds integer: {trimmed:?}"),
                    })?;
            Ok(millis / 1_000)
        },
        TimestampFormat::Rfc3339 => parse_rfc3339_to_unix_secs(trimmed),
    }
}

/// Tiny RFC 3339 parser: `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]`. Avoids
/// pulling `chrono` into `nebula-action`. Accepts the subset shipped
/// by mainstream webhook providers; rejects anything looser.
fn parse_rfc3339_to_unix_secs(s: &str) -> Result<i64, SignatureError> {
    // Split off the timezone suffix.
    let (datetime, offset_secs) = split_rfc3339_offset(s)?;

    // datetime: YYYY-MM-DDTHH:MM:SS[.fff]
    let bytes = datetime.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return Err(SignatureError::TimestampMalformed {
            reason: format!("malformed RFC 3339 timestamp: {s:?}"),
        });
    }

    let year: i64 = parse_int(&datetime[0..4], s)?;
    let month: u32 = parse_int(&datetime[5..7], s)?;
    let day: u32 = parse_int(&datetime[8..10], s)?;
    let hour: u32 = parse_int(&datetime[11..13], s)?;
    let minute: u32 = parse_int(&datetime[14..16], s)?;
    let second: u32 = parse_int(&datetime[17..19], s)?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return Err(SignatureError::TimestampMalformed {
            reason: format!("RFC 3339 fields out of range: {s:?}"),
        });
    }

    let days = days_from_civil(year, month as i32, day as i32);
    let secs = days
        .saturating_mul(86_400)
        .saturating_add(i64::from(hour) * 3_600)
        .saturating_add(i64::from(minute) * 60)
        .saturating_add(i64::from(second));
    Ok(secs - offset_secs)
}

fn split_rfc3339_offset(s: &str) -> Result<(&str, i64), SignatureError> {
    if let Some(stripped) = s.strip_suffix('Z').or_else(|| s.strip_suffix('z')) {
        return Ok((stripped, 0));
    }
    // Look for `+` or `-` after position 19 (i.e. inside the offset section).
    if s.len() < 19 {
        return Err(SignatureError::TimestampMalformed {
            reason: format!("missing timezone designator: {s:?}"),
        });
    }
    let tail = &s[19..];
    if let Some(pos) = tail.find(['+', '-']) {
        let datetime = &s[..19 + pos];
        let offset = &tail[pos..];
        if offset.len() != 6 || offset.as_bytes()[3] != b':' {
            return Err(SignatureError::TimestampMalformed {
                reason: format!("bad timezone offset: {offset:?}"),
            });
        }
        let sign: i64 = if offset.starts_with('+') { 1 } else { -1 };
        let hh: i64 = parse_int(&offset[1..3], s)?;
        let mm: i64 = parse_int(&offset[4..6], s)?;
        Ok((datetime, sign * (hh * 3_600 + mm * 60)))
    } else {
        Err(SignatureError::TimestampMalformed {
            reason: format!("missing timezone designator: {s:?}"),
        })
    }
}

fn parse_int<T: std::str::FromStr>(slice: &str, full: &str) -> Result<T, SignatureError> {
    slice
        .parse::<T>()
        .map_err(|_| SignatureError::TimestampMalformed {
            reason: format!("non-numeric component in {full:?}"),
        })
}

/// Howard Hinnant's date algorithm: days from 1970-01-01 to the
/// civil-calendar date `(y, m, d)`. Handles leap years correctly for
/// any year in `i64` range. `m` in `1..=12`; `d` in `1..=31`.
fn days_from_civil(y: i64, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe: i64 = y - era * 400; // [0, 399]
    let doy = (153 * i64::from(if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Signature encoding scheme for [`RequiredPolicy`].
///
/// Provider conventions:
/// - GitHub `X-Hub-Signature-256: sha256=<hex>` → [`Self::Sha256Hex`] (prefix accepted
///   automatically)
/// - Shopify `X-Shopify-Hmac-SHA256: <base64>` → [`Self::Sha256Base64`]
/// - Nebula canonical `X-Nebula-Signature: sha256=<hex>` → [`Self::Sha256Hex`] (default)
/// - Standard Webhooks (standardwebhooks.com) → [`Self::StandardWebhooks`]
///
/// Providers whose signature is not a raw HMAC over the request body
/// (Stripe, Slack) require [`SignaturePolicy::Custom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignatureScheme {
    /// Hex-encoded HMAC-SHA256. The value may be bare hex or prefixed
    /// with `sha256=` (GitHub convention).
    Sha256Hex,
    /// Base64-encoded HMAC-SHA256 (standard alphabet). Shopify, Square.
    Sha256Base64,
    /// Standard Webhooks (standardwebhooks.com) interoperability scheme.
    ///
    /// Uses **fixed** header names regardless of [`RequiredPolicy::header`] or
    /// [`RequiredPolicy::timestamp_header`]:
    ///
    /// | Header               | Role                         |
    /// |----------------------|------------------------------|
    /// | `webhook-id`         | Per-delivery idempotency key |
    /// | `webhook-timestamp`  | Unix-epoch seconds           |
    /// | `webhook-signature`  | Space-separated `version,base64sig` tokens |
    ///
    /// Timestamp validation is **mandatory** for this scheme (not opt-in):
    /// the replay window and future-skew cap from [`RequiredPolicy`] apply.
    ///
    /// The signed content is `{webhook-id}.{webhook-timestamp}.{raw-body}` —
    /// matching the Standard Webhooks `to_sign = "{msg_id}.{timestamp}.{payload}"`.
    ///
    /// The secret stored in [`RequiredPolicy`] is the **raw HMAC key bytes**.
    /// Decoding a `whsec_` base64-encoded secret is a registration concern;
    /// the verify path operates on already-decoded bytes.
    ///
    /// Multi-signature (`v1,<base64> v1,<base64>`) is supported for
    /// key-rotation grace: any matching `v1` token passes. Non-`v1`
    /// tokens (e.g. `v2,…`) are ignored — never selected as a weaker path.
    StandardWebhooks,
}

// ── WebhookEndpointProvider ──────────────────────────────────────────────

/// Capability injected into [`TriggerContext`] by the HTTP transport
/// layer so webhook actions can discover the public URL at which
/// external providers should POST.
///
/// Implemented by `nebula-api::webhook::EndpointProviderImpl`.
/// `nebula-action` declares the trait; it does NOT implement it —
/// the layer system forbids Business → API dependencies.
///
/// # Why a capability and not a field on `TriggerContext`
///
/// The URL shape is owned by the HTTP transport: it decides the path
/// prefix, the UUID / nonce format, whether the scheme is https, and
/// whether the host is a fixed string or a per-tenant subdomain. The
/// action has no input on any of that — it only needs to read the
/// final URL and hand it to the provider API at registration time.
/// Capability injection keeps those decisions out of the action
/// trait's signature.
///
/// # Example
///
/// ```rust,ignore
/// async fn on_activate(&self, ctx: &(impl TriggerContext + ?Sized + HasWebhookEndpoint))
/// -> Result<Self::State, ActionError>
/// {
/// let endpoint = ctx.webhook_endpoint().ok_or_else(|| {
/// ActionError::fatal("webhook trigger activated without endpoint provider")
/// })?;
/// let hook_id = github_api::create_hook(
/// &self.repo,
/// endpoint.endpoint_url().as_str(),
/// &self.secret,
/// ).await?;
/// Ok(GitHubState { hook_id })
/// }
/// ```
pub trait WebhookEndpointProvider: Send + Sync + fmt::Debug {
    /// Full public URL at which the webhook endpoint is reachable.
    ///
    /// Transport-dependent format, typically
    /// `https://<host>/<path_prefix>/<trigger_uuid>/<nonce>`. The
    /// action hands this URL to the external provider API when
    /// registering the webhook in `on_activate`.
    fn endpoint_url(&self) -> &url::Url;

    /// Path component only (no scheme / host).
    ///
    /// Useful for embedding in signed payloads where the host is
    /// not relevant, for multi-tenant routing in downstream
    /// handlers, or for structured log fields.
    fn endpoint_path(&self) -> &str;
}

// ── WebhookTriggerAdapter ────────────────────────────────────────────────────

/// Wraps a [`WebhookAction`] as a [`dyn TriggerHandler`] with state management.
///
/// Stores state from `on_activate` in a `RwLock<Option<Arc<State>>>`.
/// `handle_event` clones the `Arc` under the read lock and releases the
/// lock BEFORE awaiting `handle_request` — prevents deadlock with
/// concurrent `start`/`stop` taking a write lock (parking_lot RwLock is
/// not reentrant and not async-aware).
///
/// `handle_event` before `start()` returns `ActionError::Fatal` (no
/// silent default state).
///
/// # In-flight request tracking (M1 fix)
///
/// An atomic in-flight counter prevents `stop()` from calling
/// `on_deactivate` while `handle_request` is still running against the
/// state. `handle_event` increments before processing and decrements
/// (via RAII guard) on completion. `stop()` spins briefly if the
/// counter is non-zero.
///
/// Created automatically by `nebula_runtime::ActionRegistry::register_webhook`.
pub struct WebhookTriggerAdapter<A: WebhookAction> {
    action: A,
    /// Cached webhook-config read once from the wrapped action at
    /// construction. Owners that hand the adapter to a transport pass
    /// this to `WebhookTransport::activate` alongside the handler —
    /// it is NOT exposed through the dyn [`TriggerHandler`] contract
    /// because webhook configuration does not belong on the base
    /// trigger trait.
    config: WebhookConfig,
    meta: ActionMetadata,
    state: RwLock<Option<Arc<A::State>>>,
    /// Shared with every [`InFlightGuard`] so the guard can decrement
    /// and notify on drop without needing a lifetime back to `self`.
    in_flight: Arc<AtomicU32>,
    /// Notifies `stop()` when the in-flight counter hits zero.
    /// Replaces the old `yield_now` spin loop.
    idle_notify: Arc<Notify>,
}

impl<A: WebhookAction> WebhookTriggerAdapter<A> {
    /// Wrap a typed webhook action.
    ///
    /// Reads [`WebhookAction::config`] and [`Action::metadata`] once and
    /// caches both so the dispatch path does not re-enter the action on
    /// every request, and so the runtime / test harness can forward the
    /// policy to the transport without re-downcasting the handler.
    #[must_use]
    pub fn new(action: A) -> Self {
        let config = action.config();
        let meta = <A as Action>::metadata();
        Self {
            action,
            config,
            meta,
            state: RwLock::new(None),
            in_flight: Arc::new(AtomicU32::new(0)),
            idle_notify: Arc::new(Notify::new()),
        }
    }

    /// Webhook config cached at construction.
    ///
    /// Runtime code that wraps an action into a `WebhookTriggerAdapter`
    /// reads this and forwards it to
    /// `nebula_api::transport::webhook::WebhookTransport::activate`. Tests can
    /// also inspect it to assert the resulting policy shape.
    #[must_use]
    pub fn config(&self) -> &WebhookConfig {
        &self.config
    }
}

/// RAII guard that decrements the in-flight counter on drop and
/// wakes any `stop()` waiter when the counter transitions to zero.
struct InFlightGuard {
    counter: Arc<AtomicU32>,
    notify: Arc<Notify>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // fetch_sub returns the PREVIOUS value. If it was 1, the new
        // value is 0 — that's when a stop() waiter can proceed.
        if self.counter.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.notify.notify_waiters();
        }
    }
}

impl<A> TriggerHandler for WebhookTriggerAdapter<A>
where
    A: WebhookAction + Send + Sync + 'static,
    A::State: Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    fn start<'life0, 'life1, 'a>(
        &'life0 self,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            // Reject double-start: previous state must be stopped first.
            // Silently overwriting would leak external webhook registrations
            // (GitHub/Slack/Stripe) — the old hook stays live and stop() only
            // deactivates the last one.
            if self.state.read().is_some() {
                return Err(ActionError::fatal(
                    "webhook trigger already started; call stop() before start() again",
                ));
            }

            let new_state = self.action.on_activate(ctx).await?;

            // Re-check under the write lock to close the race between the
            // read-guard drop above and the write below. The `rollback_state`
            // dance keeps the parking_lot guard strictly inside the block so
            // it cannot sit across the `.await` on `on_deactivate` — holding
            // a non-Send, non-async guard across a suspension point would
            // make the whole future `!Send`.
            let rollback_state = {
                let mut guard = self.state.write();
                if guard.is_some() {
                    Some(new_state)
                } else {
                    *guard = Some(Arc::new(new_state));
                    None
                }
            };

            if let Some(orphan) = rollback_state {
                if let Err(e) = self.action.on_deactivate(orphan, ctx).await {
                    tracing::warn!(
                        action = %<A as Action>::metadata().base.key,
                        error = %e,
                        "webhook rollback on_deactivate failed after double-start race; \
                         external hook may leak"
                    );
                }
                return Err(ActionError::fatal(
                    "webhook trigger already started; call stop() before start() again",
                ));
            }
            Ok(())
        })
    }

    fn stop<'life0, 'life1, 'a>(
        &'life0 self,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            let stored = self.state.write().take();
            match stored {
                Some(arc_state) => {
                    // Wait for in-flight handle_event calls to complete
                    // before calling on_deactivate. Replaces the old
                    // `yield_now` busy-wait with a tokio::sync::Notify
                    // waiter — the last `InFlightGuard::drop` calls
                    // `notify_waiters` when the counter transitions to 0.
                    //
                    // The check + await pattern protects against the race
                    // where all in-flight requests finish BEFORE stop()
                    // reaches the notified() call: we check first, and
                    // only await if there's still work in flight.
                    loop {
                        if self.in_flight.load(Ordering::Acquire) == 0 {
                            break;
                        }
                        // Register interest before re-checking the counter
                        // so we don't miss the notification.
                        let notified = self.idle_notify.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        if self.in_flight.load(Ordering::Acquire) == 0 {
                            break;
                        }
                        notified.await;
                    }
                    let owned = Arc::unwrap_or_clone(arc_state);
                    self.action.on_deactivate(owned, ctx).await
                },
                None => Ok(()),
            }
        })
    }

    fn accepts_events(&self) -> bool {
        true
    }

    /// Route an incoming [`TriggerEvent`] to the typed `handle_request`.
    ///
    /// Downcasts the envelope payload to [`WebhookRequest`]. A
    /// type mismatch is an engine-level routing bug and surfaces as
    /// [`ActionError::Fatal`] — a different trigger family should
    /// never have its events delivered here.
    ///
    /// If the request carries a response channel (via
    /// [`WebhookRequest::with_response_channel`]), the adapter sends
    /// the [`WebhookHttpResponse`] through it after `handle_request`
    /// returns. If `handle_request` errors, the channel is dropped
    /// without sending — the transport receives `RecvError` and should
    /// return 500.
    ///
    /// # In-flight tracking
    ///
    /// The adapter increments an atomic counter before processing and
    /// decrements it (via RAII guard) on completion. `stop()` waits
    /// for the counter to reach zero before calling `on_deactivate`,
    /// preventing the state from being destroyed while a request is
    /// still being handled.
    fn handle_event<'life0, 'life1, 'a>(
        &'life0 self,
        event: TriggerEvent,
        ctx: &'life1 dyn TriggerContext,
    ) -> Pin<Box<dyn Future<Output = Result<TriggerEventOutcome, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            let request = match event.downcast::<WebhookRequest>() {
                Ok((_id, _received_at, req)) => req,
                Err(mismatched) => {
                    return Err(ActionError::fatal(format!(
                        "webhook adapter received non-webhook TriggerEvent payload \
                     (got {}); engine routing bug",
                        mismatched.payload_type_name()
                    )));
                },
            };

            let response_tx = request.take_response_tx();

            self.in_flight.fetch_add(1, Ordering::AcqRel);
            let _guard = InFlightGuard {
                counter: Arc::clone(&self.in_flight),
                notify: Arc::clone(&self.idle_notify),
            };

            // Clone Arc under read lock; the guard drops at end of statement BEFORE
            // the await on handle_request. Holding a parking_lot guard across.await
            // would be unsound (non-Send) and risk re-entry panic with start/stop.
            let state = if let Some(s) = self.state.read().as_ref().cloned() {
                s
            } else {
                // State could be None in two situations: (1) genuine
                // "before start" programmer error, or (2) a race with
                // stop() that already took the state. Both should not
                // hang the transport — send a 500 so the caller gets
                // a real HTTP response instead of `RecvError`.
                if let Some(tx) = response_tx {
                    let _ = tx.send(WebhookHttpResponse::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Bytes::new(),
                    ));
                }
                ctx.health().record_error();
                return Err(ActionError::fatal(
                    "handle_event called before start or after stop — no state available",
                ));
            };

            // pre_handle hook — provider-typed actions
            // (Slack url_verification, Stripe pending_webhook ping,
            // Generic GET challenge) intercept verification probes
            // here. Runs AFTER signature verification (transport
            // already enforced the policy) but BEFORE
            // `handle_request`, so the action's main path doesn't see
            // probes. `RespondNow` short-circuits with the provided
            // HTTP response and emits `TriggerEventOutcome::Skip`.
            match self.action.pre_handle(&request, ctx).await {
                Ok(PreHandleOutcome::Continue) => {},
                Ok(PreHandleOutcome::RespondNow(http_response)) => {
                    if let Some(tx) = response_tx {
                        let _ = tx.send(http_response);
                    }
                    ctx.health().record_idle();
                    return Ok(TriggerEventOutcome::Skip);
                },
                Err(e) => {
                    if let Some(tx) = response_tx {
                        let _ = tx.send(WebhookHttpResponse::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Bytes::new(),
                        ));
                    }
                    ctx.health().record_error();
                    return Err(e);
                },
            }

            // H6 — cancellation-safe dispatch. If the trigger is being
            // shut down mid-request, send `503 Service Unavailable` to
            // the external caller and return a retryable error so the
            // runtime records it and the task exits cleanly. Otherwise
            // race the handler normally.
            let response = tokio::select! {
                           biased;
                           () = ctx.cancellation().cancelled() => {
                               if let Some(tx) = response_tx {
                                   let _ = tx.send(WebhookHttpResponse::new(
                                       StatusCode::SERVICE_UNAVAILABLE,
                                       Bytes::from_static(b"shutting down"),
                                   ));
                               }
                               ctx.health().record_error();
                               return Err(ActionError::retryable(
                                   "webhook trigger cancelled mid-request",
                               ));
                           }
                           result = self.action.handle_request(&request, &state, ctx) => {
            // H1 — on handler error, send a 500 via oneshot BEFORE
            // propagating Err. Without this, the transport receives
            // RecvError and has to guess the HTTP status.
                               match result {
                                   Ok(r) => r,
                                   Err(e) => {
                                       if let Some(tx) = response_tx {
                                           let _ = tx.send(WebhookHttpResponse::new(
                                               StatusCode::INTERNAL_SERVER_ERROR,
                                               Bytes::new(),
                                           ));
                                       }
                                       ctx.health().record_error();
                                       return Err(e);
                                   }
                               }
                           }
                       };

            let (http_response, outcome) = response.into_parts();

            if let Some(tx) = response_tx {
                let _ = tx.send(http_response);
            }

            // H7 — health: emit = success, skip = idle-equivalent.
            // Matches poll's record_success/record_idle split.
            match &outcome {
                TriggerEventOutcome::Emit(_) => ctx.health().record_success(1),
                TriggerEventOutcome::EmitMany(batch) => {
                    ctx.health().record_success(batch.len() as u64);
                },
                TriggerEventOutcome::Skip => ctx.health().record_idle(),
            }

            Ok(outcome)
        })
    }
}

impl<A: WebhookAction> fmt::Debug for WebhookTriggerAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookTriggerAdapter")
            .field("action", &<A as Action>::metadata().base.key)
            .finish_non_exhaustive()
    }
}

// ── HMAC signature primitives ────────────────────────────────────────────────

type HmacSha256 = Hmac<Sha256>;

/// Outcome of a signature verification attempt.
///
/// `Missing` and `Invalid` are distinct so callers can decide policy:
/// a multi-tenant webhook endpoint may want to `Skip` on `Missing`
/// (not our event) but `Skip` on `Invalid` too (tampered), while a
/// strict endpoint may want to log or reject on `Invalid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignatureOutcome {
    /// Signature header present and matches the computed HMAC.
    Valid,
    /// Signature header is absent from the request.
    Missing,
    /// Signature header is present but does not match — bad hex, wrong
    /// length, or mismatched digest.
    Invalid,
}

impl SignatureOutcome {
    /// `true` only if the signature was present AND matched.
    ///
    /// Use this as the default "is it safe to emit the event" guard:
    ///
    /// ```ignore
    /// if !verify_hmac_sha256(request, secret, "X-Hub-Signature-256")?.is_valid() {
    /// return Ok(TriggerEventOutcome::skip());
    /// }
    /// ```
    #[must_use]
    pub fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// Pre-scan JSON bytes for maximum nesting depth.
///
/// Single-pass byte scan that tracks `{`/`[` vs `}`/`]` depth,
/// skipping the contents of JSON strings (with `\\` escape
/// handling). Returns an error synthesised via [`serde::de::Error`]
/// so `body_json_bounded` can report a uniform error type.
///
/// This is NOT a full JSON validator — malformed input will fall
/// through to `serde_json` for a real error report. The only thing
/// this guards is stack-overflow from pathologically nested input
/// that would otherwise blow the tokio worker stack.
fn check_json_depth(bytes: &[u8], max_depth: usize) -> Result<(), serde_json::Error> {
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escape = false;

    for &b in bytes {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {},
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                if depth > max_depth {
                    return Err(<serde_json::Error as serde::de::Error>::custom(format!(
                        "webhook body JSON exceeds max depth {max_depth}"
                    )));
                }
            },
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
            },
            _ => {},
        }
    }
    Ok(())
}

/// Result of a strict single-value header lookup.
///
/// Used by signature verification to reject proxy-chain header
/// duplication attacks (H3 in the webhook hardening audit).
enum HeaderLookup<'a> {
    Missing,
    One(&'a str),
    Multiple,
}

/// Look up a header and require exactly one value.
///
/// Returns `Multiple` if two or more header values are present with
/// the same name, or if the single value is not valid ASCII.
fn single_header_value<'a>(headers: &'a HeaderMap, name: &HeaderName) -> HeaderLookup<'a> {
    let mut iter = headers.get_all(name).iter();
    let Some(first) = iter.next() else {
        return HeaderLookup::Missing;
    };
    if iter.next().is_some() {
        return HeaderLookup::Multiple;
    }
    match first.to_str() {
        Ok(s) => HeaderLookup::One(s),
        Err(_) => HeaderLookup::Multiple,
    }
}

/// Build an `HmacSha256` over `body`. Panic-free by construction:
/// HMAC (RFC 2104) accepts any key length, so `new_from_slice` only
/// fails for block-cipher MACs that don't apply here.
fn hmac_sha256_over(secret: &[u8], body: &[u8]) -> HmacSha256 {
    #[expect(
        clippy::expect_used,
        reason = "HMAC (RFC 2104) accepts any key length; new_from_slice only fails for block-cipher MACs"
    )]
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length (RFC 2104)");
    mac.update(body);
    mac
}

/// Timing-invariant hex-HMAC verification (H9).
///
/// Always runs the full MAC computation, regardless of whether the
/// incoming hex decodes successfully. This avoids a data-dependent
/// early return that could leak header well-formedness via timing.
/// A decode failure substitutes a zero-filled 32-byte expected tag;
/// the constant-time compare then fails, and the final result is
/// `Invalid` — identical in duration to a valid-hex mismatch.
fn verify_hex_hmac_timing_invariant(
    secret: &[u8],
    body: &[u8],
    sig_header: &str,
) -> SignatureOutcome {
    // Strip the GitHub-style prefix if present.
    let sig_hex = sig_header
        .strip_prefix("sha256=")
        .unwrap_or(sig_header)
        .trim();

    let (expected, decode_ok) = match hex::decode(sig_hex) {
        Ok(v) => (v, true),
        Err(_) => (vec![0u8; 32], false),
    };

    let mac = hmac_sha256_over(secret, body);
    let compare_ok = mac.verify_slice(&expected).is_ok();

    if decode_ok && compare_ok {
        SignatureOutcome::Valid
    } else {
        SignatureOutcome::Invalid
    }
}

/// Timing-invariant base64-HMAC verification — H4 sibling helper.
///
/// Same discipline as the hex variant: always runs the MAC even
/// when base64 decode fails. Shopify and Square use this scheme
/// (raw 32-byte HMAC, standard base64 alphabet, no prefix).
fn verify_base64_hmac_timing_invariant(
    secret: &[u8],
    body: &[u8],
    sig_header: &str,
) -> SignatureOutcome {
    let trimmed = sig_header.trim();
    let (expected, decode_ok) = match BASE64_STANDARD.decode(trimmed) {
        Ok(v) => (v, true),
        Err(_) => (vec![0u8; 32], false),
    };

    let mac = hmac_sha256_over(secret, body);
    let compare_ok = mac.verify_slice(&expected).is_ok();

    if decode_ok && compare_ok {
        SignatureOutcome::Valid
    } else {
        SignatureOutcome::Invalid
    }
}

/// Verify an HMAC-SHA256 signature from a named header against the
/// webhook request body.
///
/// Accepts either bare hex (`"abcd1234…"`) or a prefixed form
/// (`"sha256=abcd…"`). The prefix, if any, is stripped before hex
/// decoding.
///
/// # Arguments
///
/// - `request` — the incoming webhook request (body + headers)
/// - `secret` — shared HMAC key (typically from a credential)
/// - `header` — header name carrying the signature, e.g. `"X-Hub-Signature-256"`. Must be a valid
///   HTTP header name.
///
/// Header lookup is case-insensitive (delegated to `HeaderMap`).
///
/// # Returns
///
/// [`SignatureOutcome::Valid`] / `Missing` / `Invalid`. Never panics,
/// never leaks length via timing — digest comparison delegates to
/// [`hmac::Mac::verify_slice`] which uses `subtle::ConstantTimeEq`.
///
/// # Errors
///
/// Returns [`ActionError::Validation`] if:
/// - `secret` is empty (silently produces a valid MAC for any input — almost always a
///   misconfiguration, fail-closed).
/// - `header` is not a valid HTTP header name.
pub fn verify_hmac_sha256(
    request: &WebhookRequest,
    secret: &[u8],
    header: &str,
) -> Result<SignatureOutcome, ActionError> {
    if secret.is_empty() {
        return Err(ActionError::validation(
            "webhook.secret",
            ValidationReason::MissingField,
            Some("webhook signature verification requires a non-empty HMAC secret".to_string()),
        ));
    }

    let name = HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
        ActionError::validation(
            "webhook.signature_header",
            ValidationReason::WrongType,
            Some(format!("invalid HTTP header name: {header:?}")),
        )
    })?;

    // H3 — strict single-valued signature header. A proxy chain that
    // appends rather than replaces can produce duplicate headers;
    // picking "the first" gives an attacker-controlled slot. Reject
    // anything other than exactly one value.
    let sig_header = match single_header_value(request.headers(), &name) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Ok(SignatureOutcome::Missing),
        HeaderLookup::Multiple => return Ok(SignatureOutcome::Invalid),
    };

    Ok(verify_hex_hmac_timing_invariant(
        secret,
        request.body(),
        sig_header,
    ))
}

/// Verify an HMAC-SHA256 signature where the header value is
/// base64-encoded instead of hex-encoded. Covers Shopify and Square.
///
/// Identical discipline to [`verify_hmac_sha256`]:
/// - Strict single-valued signature header
/// - Constant-time compare via [`hmac::Mac::verify_slice`]
/// - Timing-invariant decode + MAC sequence
/// - Empty secret → `Err(Validation)` (fail-closed)
///
/// # Errors
///
/// Same as [`verify_hmac_sha256`] — empty secret or invalid HTTP
/// header name produce `ActionError::Validation`. Missing header is
/// `Ok(Missing)`, not `Err`.
pub fn verify_hmac_sha256_base64(
    request: &WebhookRequest,
    secret: &[u8],
    header: &str,
) -> Result<SignatureOutcome, ActionError> {
    if secret.is_empty() {
        return Err(ActionError::validation(
            "webhook.secret",
            ValidationReason::MissingField,
            Some("webhook signature verification requires a non-empty HMAC secret".to_string()),
        ));
    }

    let name = HeaderName::from_bytes(header.as_bytes()).map_err(|_| {
        ActionError::validation(
            "webhook.signature_header",
            ValidationReason::WrongType,
            Some(format!("invalid HTTP header name: {header:?}")),
        )
    })?;

    let sig_header = match single_header_value(request.headers(), &name) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Ok(SignatureOutcome::Missing),
        HeaderLookup::Multiple => return Ok(SignatureOutcome::Invalid),
    };

    Ok(verify_base64_hmac_timing_invariant(
        secret,
        request.body(),
        sig_header,
    ))
}

/// Verify an HMAC-SHA256 signature over a canonical payload derived
/// from a timestamp header and the request body — covers Stripe and
/// Slack replay-protection schemes.
///
/// Unlike [`verify_hmac_sha256`], this helper enforces a replay
/// window: the timestamp carried in `ts_header` must be within
/// `tolerance` of the request's arrival time (measured from
/// [`WebhookRequest::received_at`], never the wall clock). This
/// prevents an attacker from replaying a captured signed request
/// hours later and having it accepted.
///
/// The caller supplies a `canonicalize` closure that builds the
/// byte slice that was HMAC'd. This lets one helper serve multiple
/// provider schemes:
///
/// - **Stripe** signs `{ts}.{body_utf8}`: `|ts, body| format!("{ts}.{}",
///   std::str::from_utf8(body).unwrap_or_default()).into_bytes()`. The outer header is
///   `Stripe-Signature: t=<ts>,v1=<hex>`; extract `t` and `v1` in user code, then call this helper
///   with `sig_header` set to a header you stored the parsed `v1` into, or use
///   [`hmac_sha256_compute`] + [`verify_tag_constant_time`] directly.
/// - **Slack** signs `v0:{ts}:{body_utf8}`: `|ts, body| format!("v0:{ts}:{}",
///   std::str::from_utf8(body).unwrap_or_default()).into_bytes()`.
///
/// # Clock source
///
/// This helper reads [`WebhookRequest::received_at`], set by the
/// transport when the HTTP request was received. It **never** reads
/// the wall clock — replay verification is deterministic for any
/// given `WebhookRequest`, which is important for replay from
/// persisted events.
///
/// Future timestamps (outside `tolerance`, or more than 60 s ahead
/// of `received_at`) are rejected as `Invalid`.
///
/// # Errors
///
/// - `ActionError::Validation` if `secret` is empty, or if either `sig_header` or `ts_header` is
///   not a valid HTTP header name.
pub fn verify_hmac_sha256_with_timestamp(
    request: &WebhookRequest,
    secret: &[u8],
    sig_header: &str,
    ts_header: &str,
    tolerance: Duration,
    canonicalize: impl Fn(&str, &[u8]) -> Vec<u8>,
) -> Result<SignatureOutcome, ActionError> {
    if secret.is_empty() {
        return Err(ActionError::validation(
            "webhook.secret",
            ValidationReason::MissingField,
            Some("webhook signature verification requires a non-empty HMAC secret".to_string()),
        ));
    }

    let sig_name = HeaderName::from_bytes(sig_header.as_bytes()).map_err(|_| {
        ActionError::validation(
            "webhook.signature_header",
            ValidationReason::WrongType,
            Some(format!("invalid HTTP header name: {sig_header:?}")),
        )
    })?;
    let ts_name = HeaderName::from_bytes(ts_header.as_bytes()).map_err(|_| {
        ActionError::validation(
            "webhook.timestamp_header",
            ValidationReason::WrongType,
            Some(format!("invalid HTTP header name: {ts_header:?}")),
        )
    })?;

    let ts_str = match single_header_value(request.headers(), &ts_name) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Ok(SignatureOutcome::Missing),
        HeaderLookup::Multiple => return Ok(SignatureOutcome::Invalid),
    };
    let sig_str = match single_header_value(request.headers(), &sig_name) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Ok(SignatureOutcome::Missing),
        HeaderLookup::Multiple => return Ok(SignatureOutcome::Invalid),
    };

    // Replay window: timestamp must be within tolerance of received_at.
    let ts_secs: i64 = match ts_str.trim().parse() {
        Ok(n) => n,
        Err(_) => return Ok(SignatureOutcome::Invalid),
    };
    let received_secs: i64 = match request.received_at().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return Ok(SignatureOutcome::Invalid),
    };
    let skew = received_secs.saturating_sub(ts_secs);
    let tol_secs = tolerance.as_secs() as i64;
    // Accept [received - tol, received + 60]. Allow a small positive
    // forward skew for clock drift, but reject signatures dated far
    // in the future — a replay window that accepts far-future
    // timestamps is no replay window at all.
    const FUTURE_SKEW_SECS: i64 = 60;
    if skew > tol_secs || skew < -FUTURE_SKEW_SECS {
        return Ok(SignatureOutcome::Invalid);
    }

    // Build canonical signed payload via user-supplied closure.
    let canonical = canonicalize(ts_str, request.body());

    Ok(verify_hex_hmac_timing_invariant(
        secret, &canonical, sig_str,
    ))
}

/// Standard Webhooks (standardwebhooks.com) signature verification.
///
/// Called by [`RequiredPolicy::verify_with`] when the scheme is
/// [`SignatureScheme::StandardWebhooks`].
///
/// # Fixed header names
///
/// This function ignores [`RequiredPolicy::header`] and
/// [`RequiredPolicy::timestamp_header`]. Standard Webhooks mandates:
///
/// | Header               | Role                                               |
/// |----------------------|----------------------------------------------------|
/// | `webhook-id`         | Per-delivery idempotency key (NOT a secret)        |
/// | `webhook-timestamp`  | Unix-epoch seconds (mandatory)                     |
/// | `webhook-signature`  | Space-separated `version,base64sig` tokens         |
///
/// # Signed content
///
/// The bytes fed to HMAC-SHA256 are `{webhook-id}.{webhook-timestamp}.{raw-body}`,
/// matching the Standard Webhooks spec's `to_sign = "{msg_id}.{timestamp}.{payload}"`.
///
/// # Secret
///
/// `secret` is the **raw HMAC key bytes**. `whsec_` prefix decoding is a
/// registration concern; verify operates on already-decoded bytes.
///
/// # Multi-signature / key rotation
///
/// `webhook-signature` is a SPACE-separated list of `version,base64sig` tokens.
/// Any `v1` token whose decoded bytes constant-time-match the computed MAC passes.
/// Non-`v1` tokens are ignored (algorithm-confusion guard — never fall through to a
/// weaker path). A decode failure for a candidate token is a non-match, not an error.
/// Absence of any `v1` token → `Missing`; all `v1` tokens fail → `Invalid`.
///
/// # Errors
///
/// Returns [`SignatureError`] when:
/// - `webhook-timestamp` is absent → `TimestampMissing`
/// - `webhook-timestamp` is malformed / outside the replay window → timestamp errors
///
/// Absent or all-failed `webhook-signature` returns `Ok(Missing/Invalid)`.
fn verify_standard_webhooks(
    request: &WebhookRequest,
    secret: &[u8],
    replay_window: Duration,
    clock: &dyn Clock,
) -> Result<SignatureOutcome, SignatureError> {
    // Fixed Standard Webhooks header names (spec §4).
    const ID_HEADER: HeaderName = HeaderName::from_static("webhook-id");
    const TS_HEADER: HeaderName = HeaderName::from_static("webhook-timestamp");
    const SIG_HEADER: HeaderName = HeaderName::from_static("webhook-signature");

    // 1. Extract webhook-id.
    let id_str = match single_header_value(request.headers(), &ID_HEADER) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => {
            // No id → treat as Missing (signature cannot be verified without id).
            return Ok(SignatureOutcome::Missing);
        },
        HeaderLookup::Multiple => return Ok(SignatureOutcome::Invalid),
    };

    // 2. Extract webhook-timestamp — mandatory for StandardWebhooks.
    let ts_str = match single_header_value(request.headers(), &TS_HEADER) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Err(SignatureError::TimestampMissing),
        HeaderLookup::Multiple => {
            return Err(SignatureError::TimestampMalformed {
                reason: "multiple webhook-timestamp headers".to_string(),
            });
        },
    };

    // 3. Validate timestamp (reuse the shared helper that understands replay_window
    //    and FUTURE_SKEW_SECS). Format is always Unix seconds for Standard Webhooks.
    validate_timestamp(
        request.headers(),
        &TS_HEADER,
        TimestampFormat::UnixSeconds,
        replay_window,
        clock,
    )?;

    // 4. Build signed content: "{webhook-id}.{webhook-timestamp}.{body}".
    //    Allocate once; avoid a format! with a Copy-heavy body clone.
    let prefix = format!("{id_str}.{ts_str}.");
    let mut signed_content = Vec::with_capacity(prefix.len() + request.body().len());
    signed_content.extend_from_slice(prefix.as_bytes());
    signed_content.extend_from_slice(request.body());

    // 5. Compute expected MAC over the signed content. Done ONCE before any
    //    candidate loop — ensures constant MAC-compute time regardless of how
    //    many candidates pass or fail.
    let expected_mac: [u8; 32] = hmac_sha256_compute(secret, &signed_content);

    // 6. Parse webhook-signature header: SPACE-separated "version,base64" tokens.
    let sig_header_str = match single_header_value(request.headers(), &SIG_HEADER) {
        HeaderLookup::One(v) => v,
        HeaderLookup::Missing => return Ok(SignatureOutcome::Missing),
        HeaderLookup::Multiple => return Ok(SignatureOutcome::Invalid),
    };

    let mut any_v1_candidate = false;
    for token in sig_header_str.split_ascii_whitespace() {
        // Each token is "version,base64sig".
        let Some((version, b64_sig)) = token.split_once(',') else {
            // Malformed token — ignore (not a valid candidate, not a hard failure).
            continue;
        };
        if version != "v1" {
            // Non-v1 algorithm — algorithm-confusion guard: skip entirely.
            // Never select a weaker or unknown path.
            continue;
        }
        any_v1_candidate = true;
        // Decode — failure is a non-match (substitute zeroes for constant time).
        let (candidate_bytes, decode_ok) = match BASE64_STANDARD.decode(b64_sig) {
            Ok(v) => (v, true),
            Err(_) => (vec![0u8; 32], false),
        };
        // Constant-time compare.
        if decode_ok && verify_tag_constant_time(&expected_mac, &candidate_bytes) {
            return Ok(SignatureOutcome::Valid);
        }
    }

    if any_v1_candidate {
        // v1 tokens present but none matched.
        Ok(SignatureOutcome::Invalid)
    } else {
        // No v1 token at all — treat as Missing.
        Ok(SignatureOutcome::Missing)
    }
}

/// Compute a raw HMAC-SHA256 tag over arbitrary bytes.
///
/// Escape hatch for signature schemes not handled by
/// [`verify_hmac_sha256`]. Build the signed payload yourself (for
/// example, Stripe's `{timestamp}.{body}` or Slack's
/// `v0:{timestamp}:{body}`), then compare the result against the
/// header-provided tag with [`verify_tag_constant_time`].
///
/// # Panics
///
/// Never — `Hmac::new_from_slice` accepts any key length for HMAC.
#[must_use]
pub fn hmac_sha256_compute(secret: &[u8], payload: &[u8]) -> [u8; 32] {
    // Reason: see `verify_hmac_sha256` — HMAC construction is
    // structurally infallible (RFC 2104). Returning `Result` from a
    // pure primitive for an unreachable error case would force every
    // caller to handle an impossibility.
    #[expect(
        clippy::expect_used,
        reason = "HMAC construction is structurally infallible (RFC 2104); Result would force callers to handle an impossibility"
    )]
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length (RFC 2104)");
    mac.update(payload);
    mac.finalize().into_bytes().into()
}

/// Constant-time tag comparison.
///
/// Use with [`hmac_sha256_compute`] for custom signature schemes.
/// Delegates to `subtle::ConstantTimeEq`; returns `false` on length
/// mismatch without branching on content, so neither the length nor
/// the bytes leak via timing.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_action::webhook::{hmac_sha256_compute, verify_tag_constant_time};
///
/// // Stripe-style "t=…,v1=…" signature.
/// let signed_payload = format!("{timestamp}.{}", std::str::from_utf8(body).unwrap());
/// let expected = hmac_sha256_compute(secret, signed_payload.as_bytes());
/// let provided = hex::decode(header_v1).unwrap_or_default();
/// if !verify_tag_constant_time(&expected, &provided) {
/// return Ok(TriggerEventOutcome::skip());
/// }
/// ```
#[must_use]
pub fn verify_tag_constant_time(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

// ── Test helpers ────────────────────────────────────────────────────────────

/// Convenience builder used by integration tests — builds a `WebhookRequest`
/// from a raw body and a slice of `(name, value)` header pairs without
/// making the caller plumb `http::HeaderMap` manually.
///
/// `#[doc(hidden)]` because the production path is
/// [`WebhookRequest::try_new`]; this helper only exists so unit and
/// integration tests don't have to rewrite the same header-building
/// boilerplate.
///
/// # Errors
///
/// Same as [`WebhookRequest::try_new`]; additionally returns
/// [`ActionError::Validation`] if any header name is not a valid HTTP
/// header name or any value contains disallowed bytes.
#[doc(hidden)]
pub fn webhook_request_for_test(
    body: &[u8],
    headers: &[(&str, &str)],
) -> Result<WebhookRequest, ActionError> {
    webhook_request_for_test_with_limits(body, headers, DEFAULT_MAX_BODY_BYTES, MAX_HEADER_COUNT)
}

/// As [`webhook_request_for_test`], but with custom limits. Used by the
/// integration tests that exercise cap boundaries.
#[doc(hidden)]
pub fn webhook_request_for_test_with_limits(
    body: &[u8],
    headers: &[(&str, &str)],
    max_body: usize,
    max_headers: usize,
) -> Result<WebhookRequest, ActionError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (k, v) in headers {
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|_| {
            ActionError::validation(
                "headers",
                ValidationReason::WrongType,
                Some(format!("invalid header name: {k:?}")),
            )
        })?;
        let value = HeaderValue::from_str(v).map_err(|_| {
            ActionError::validation(
                "headers",
                ValidationReason::WrongType,
                Some(format!("invalid header value for {k:?}")),
            )
        })?;
        map.append(name, value);
    }

    WebhookRequest::try_new_with_limits(
        Method::POST,
        "/webhook".to_string(),
        None::<String>,
        map,
        Bytes::copy_from_slice(body),
        max_body,
        max_headers,
    )
}

// ── Unit tests: Standard Webhooks HMAC scheme ─────────────────────────────────

#[cfg(test)]
mod swh_tests {
    //! Unit tests for [`SignatureScheme::StandardWebhooks`] via
    //! [`RequiredPolicy::verify_with`].
    //!
    //! All tests use a deterministic [`MockClock`] so replay-window
    //! assertions are immune to wall-clock drift.

    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

    use super::*;

    /// Fixed Unix-epoch anchor shared across tests.
    const NOW_SECS: u64 = 1_700_000_000;

    /// Test HMAC key (raw bytes — no whsec_ prefix).
    const KEY: &[u8] = b"test-secret-key-for-standard-webhooks";

    /// Build a [`RequiredPolicy`] configured for Standard Webhooks.
    fn swh_policy() -> RequiredPolicy {
        RequiredPolicy::new()
            .with_secret(KEY)
            .with_scheme(SignatureScheme::StandardWebhooks)
    }

    /// Compute a valid Standard Webhooks signature token for the given
    /// message-id, timestamp-string, and body.
    ///
    /// Returns a `webhook-signature` header value of the form `v1,<base64>`.
    fn sign(msg_id: &str, ts_str: &str, body: &[u8]) -> String {
        let prefix = format!("{msg_id}.{ts_str}.");
        let mut content = Vec::with_capacity(prefix.len() + body.len());
        content.extend_from_slice(prefix.as_bytes());
        content.extend_from_slice(body);
        let mac = hmac_sha256_compute(KEY, &content);
        format!("v1,{}", B64.encode(mac))
    }

    fn make_request(msg_id: &str, ts_str: &str, sig: &str, body: &[u8]) -> WebhookRequest {
        webhook_request_for_test(
            body,
            &[
                ("webhook-id", msg_id),
                ("webhook-timestamp", ts_str),
                ("webhook-signature", sig),
            ],
        )
        .expect("test request must be constructable")
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    /// Valid Standard Webhooks request → `Ok(())`.
    #[test]
    fn swh_valid_signature_passes() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let body = b"{\"event\":\"test\"}";
        let sig = sign("msg_abc123", &ts, body);
        let req = make_request("msg_abc123", &ts, &sig, body);
        swh_policy()
            .verify_with(&req, &clock)
            .expect("valid SWH signature must pass");
    }

    // ── Tampering ─────────────────────────────────────────────────────────────

    /// Tampered body → signature mismatch → `SignatureInvalid`.
    ///
    /// Red-on-revert: removing the body from the signed content makes a
    /// body-tamper undetectable.
    #[test]
    fn swh_tampered_body_is_invalid() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let original_body = b"{\"event\":\"test\"}";
        let tampered_body = b"{\"event\":\"hacked\"}";
        let sig = sign("msg_abc123", &ts, original_body);
        // Request body differs from what was signed.
        let req = make_request("msg_abc123", &ts, &sig, tampered_body);
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("tampered body must fail verification");
        assert!(
            matches!(err, SignatureError::SignatureInvalid),
            "expected SignatureInvalid, got {err:?}"
        );
    }

    /// Tampered timestamp → signature mismatch → `SignatureInvalid`.
    ///
    /// The timestamp is part of the signed content, so changing it after
    /// signing invalidates the MAC.
    #[test]
    fn swh_tampered_timestamp_is_invalid() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let tampered_ts = (NOW_SECS - 10).to_string(); // different ts sent in header
        let body = b"{\"event\":\"test\"}";
        let sig = sign("msg_abc123", &ts, body);
        // Header has a different timestamp than what was signed.
        let req = make_request("msg_abc123", &tampered_ts, &sig, body);
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("tampered timestamp must fail verification");
        assert!(
            matches!(err, SignatureError::SignatureInvalid),
            "expected SignatureInvalid, got {err:?}"
        );
    }

    // ── Replay window ─────────────────────────────────────────────────────────

    /// Timestamp older than the replay window → `TimestampOutOfWindow`.
    #[test]
    fn swh_expired_timestamp_rejected() {
        // Advance clock 6 minutes past the request timestamp.
        let req_ts = NOW_SECS;
        let clock_now = NOW_SECS + 360; // 6 min later, outside 5-min window
        let clock = MockClock::at_unix_secs(clock_now);
        let ts = req_ts.to_string();
        let body = b"{}";
        let sig = sign("msg_exp", &ts, body);
        let req = make_request("msg_exp", &ts, &sig, body);
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("expired timestamp must be rejected");
        assert!(
            matches!(err, SignatureError::TimestampOutOfWindow { .. }),
            "expected TimestampOutOfWindow, got {err:?}"
        );
    }

    /// Timestamp more than 60 s in the future → `TimestampOutOfWindow`.
    ///
    /// The future-skew cap (`FUTURE_SKEW_SECS = 60`) applies independently of
    /// the configured replay window, preventing tokens pre-dated in the future.
    #[test]
    fn swh_future_skew_beyond_cap_rejected() {
        let req_ts = NOW_SECS + 120; // 2 min ahead — exceeds the 60-s cap
        let clock_now = NOW_SECS;
        let clock = MockClock::at_unix_secs(clock_now);
        let ts = req_ts.to_string();
        let body = b"{}";
        let sig = sign("msg_future", &ts, body);
        let req = make_request("msg_future", &ts, &sig, body);
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("far-future timestamp must be rejected");
        assert!(
            matches!(err, SignatureError::TimestampOutOfWindow { .. }),
            "expected TimestampOutOfWindow, got {err:?}"
        );
    }

    // ── Missing headers ───────────────────────────────────────────────────────

    /// Missing `webhook-timestamp` → `TimestampMissing` (mandatory for SWH).
    #[test]
    fn swh_missing_timestamp_is_error() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let body = b"{}";
        let sig = sign("msg_nots", &ts, body);
        // No webhook-timestamp header.
        let req = webhook_request_for_test(
            body,
            &[("webhook-id", "msg_nots"), ("webhook-signature", &sig)],
        )
        .expect("request build");
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("missing timestamp must error");
        assert!(
            matches!(err, SignatureError::TimestampMissing),
            "expected TimestampMissing, got {err:?}"
        );
    }

    /// Missing `webhook-signature` → `SignatureMissing`.
    #[test]
    fn swh_missing_signature_header() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let body = b"{}";
        // No webhook-signature header.
        let req = webhook_request_for_test(
            body,
            &[("webhook-id", "msg_nosig"), ("webhook-timestamp", &ts)],
        )
        .expect("request build");
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("missing signature must error");
        assert!(
            matches!(err, SignatureError::SignatureMissing),
            "expected SignatureMissing, got {err:?}"
        );
    }

    // ── Multi-signature scenarios ─────────────────────────────────────────────

    /// `v1,<bad> v1,<good>` → first candidate fails, second succeeds → `Valid`.
    ///
    /// Proves the candidate loop does not short-circuit on a bad token.
    #[test]
    fn swh_multi_sig_bad_then_good_passes() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let body = b"{}";
        let good = sign("msg_multi", &ts, body);
        let bad = "v1,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let sig_header = format!("{bad} {good}");
        let req = make_request("msg_multi", &ts, &sig_header, body);
        swh_policy()
            .verify_with(&req, &clock)
            .expect("bad-then-good multi-sig must pass");
    }

    /// `v2,<x> v1,<good>` → non-v1 ignored, v1 accepted → `Valid`.
    ///
    /// Proves algorithm-confusion guard: `v2` is never selected even if it
    /// appears first.
    #[test]
    fn swh_v2_ignored_v1_accepted() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let body = b"{}";
        let good = sign("msg_v2v1", &ts, body);
        let sig_header = format!("v2,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= {good}");
        let req = make_request("msg_v2v1", &ts, &sig_header, body);
        swh_policy()
            .verify_with(&req, &clock)
            .expect("v2 must be ignored; v1 must pass");
    }

    /// Only `v2,<x>` token present → no v1 candidate → `SignatureMissing`.
    #[test]
    fn swh_only_v2_no_v1_is_missing() {
        let clock = MockClock::at_unix_secs(NOW_SECS);
        let ts = NOW_SECS.to_string();
        let body = b"{}";
        let sig_header = "v2,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let req = make_request("msg_v2only", &ts, sig_header, body);
        let err = swh_policy()
            .verify_with(&req, &clock)
            .expect_err("only-v2 must be SignatureMissing");
        assert!(
            matches!(err, SignatureError::SignatureMissing),
            "expected SignatureMissing, got {err:?}"
        );
    }
}
