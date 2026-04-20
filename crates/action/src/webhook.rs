//! Webhook trigger domain — DX trait, adapter, HMAC signature primitives,
//! and the canonical HTTP-shaped event type ([`WebhookRequest`]).
//!
//! Groups everything an action author needs to implement a webhook
//! trigger into one file:
//!
//! - [`WebhookRequest`] — typed HTTP event (method, path, query, headers, body, arrival timestamp).
//!   Constructed by the HTTP transport layer via [`WebhookRequest::try_new`] /
//!   [`WebhookRequest::try_new_with_limits`] which enforce body-size and header-count limits at the
//!   boundary.
//! - [`WebhookAction`] — DX trait with the register / handle / unregister lifecycle. Implement this
//!   and register via `registry.register_webhook(action)`.
//! - [`WebhookTriggerAdapter`] — bridges a typed `WebhookAction` to the
//!   [`TriggerHandler`](crate::trigger::TriggerHandler) dyn contract. Stores state from
//!   `on_activate` in a `RwLock<Option<Arc<State>>>`, rejects double-start with
//!   `ActionError::Fatal`, cleans up orphaned state on lost-race rollback, and downcasts incoming
//!   [`TriggerEvent`](crate::trigger::TriggerEvent)s to [`WebhookRequest`].
//! - [`verify_hmac_sha256`], [`hmac_sha256_compute`], [`verify_tag_constant_time`],
//!   [`SignatureOutcome`] — constant-time HMAC primitives. Use `verify_hmac_sha256` for
//!   GitHub-style `sha256=…` bare-hex signatures; reach for the lower-level pair for Stripe / Slack
//!   schemes that sign a derived payload.
//! - [`WebhookConfig`], [`SignaturePolicy`], [`RequiredPolicy`], [`SignatureScheme`] — the
//!   `Required`-by-default signature-enforcement contract at the trait surface. Authors return a
//!   [`WebhookConfig`] from [`WebhookAction::config`]; the HTTP transport reads its
//!   [`WebhookConfig::signature_policy`] before dispatch; misconfigured or unsigned requests are
//!   rejected before the action sees them. See ADR-0022.
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

use std::{
    fmt,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use bytes::Bytes;
use hmac::{Hmac, KeyInit, Mac};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use parking_lot::RwLock;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, oneshot};

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
///     StatusCode::OK,
///     challenge_value,
///     TriggerEventOutcome::skip(),
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
///     type State = WebhookReg;
///
///     async fn on_activate(&self, ctx: &TriggerContext) -> Result<WebhookReg, ActionError> {
///         Ok(WebhookReg { hook_id: register(ctx).await? })
///     }
///
///     async fn handle_request(&self, req: &WebhookRequest, _state: &Self::State, _ctx: &TriggerContext)
///         -> Result<WebhookResponse, ActionError> {
///         let outcome = verify_hmac_sha256(req, &self.secret, "X-Hub-Signature-256")?;
///         if !outcome.is_valid() {
///             return Ok(WebhookResponse::accept(TriggerEventOutcome::skip()));
///         }
///         Ok(WebhookResponse::accept(TriggerEventOutcome::emit(req.body_json()?)))
///     }
///
///     async fn on_deactivate(&self, state: WebhookReg, _ctx: &TriggerContext) -> Result<(), ActionError> {
///         delete_hook(&state.hook_id).await
///     }
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
        ctx: &TriggerContext,
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
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<WebhookResponse, ActionError>> + Send;

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
        _ctx: &TriggerContext,
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
    /// See ADR-0022 for the full rationale.
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
/// - [`signature_policy`](Self::signature_policy) — ADR-0022 signature enforcement policy. Default:
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

    /// Signature policy enforced by the transport before dispatch.
    #[must_use]
    pub fn signature_policy(&self) -> &SignaturePolicy {
        &self.signature_policy
    }
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
/// See ADR-0022 for the full rationale.
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
/// supply a secret" at the transport layer. See ADR-0022.
#[derive(Clone)]
pub struct RequiredPolicy {
    secret: Arc<[u8]>,
    header: HeaderName,
    scheme: SignatureScheme,
}

impl RequiredPolicy {
    /// Canonical Nebula default: `X-Nebula-Signature`,
    /// [`SignatureScheme::Sha256Hex`], empty secret (fail-closed until
    /// an author supplies one).
    #[must_use]
    pub fn new() -> Self {
        Self {
            secret: Arc::from(Vec::<u8>::new()),
            header: HeaderName::from_static("x-nebula-signature"),
            scheme: SignatureScheme::Sha256Hex,
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
            .finish()
    }
}

/// Signature encoding scheme for [`RequiredPolicy`].
///
/// Provider conventions:
/// - GitHub `X-Hub-Signature-256: sha256=<hex>` → [`Self::Sha256Hex`] (prefix accepted
///   automatically)
/// - Shopify `X-Shopify-Hmac-SHA256: <base64>` → [`Self::Sha256Base64`]
/// - Nebula canonical `X-Nebula-Signature: sha256=<hex>` → [`Self::Sha256Hex`] (default)
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
/// async fn on_activate(&self, ctx: &TriggerContext)
///     -> Result<Self::State, ActionError>
/// {
///     let endpoint = ctx.webhook.as_ref().ok_or_else(|| {
///         ActionError::fatal("webhook trigger activated without endpoint provider")
///     })?;
///     let hook_id = github_api::create_hook(
///         &self.repo,
///         endpoint.endpoint_url().as_str(),
///         &self.secret,
///     ).await?;
///     Ok(GitHubState { hook_id })
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
    /// Reads [`WebhookAction::config`] once and caches the result so
    /// the dispatch path does not re-enter the action on every
    /// request, and so the runtime / test harness can forward the
    /// policy to the transport without re-downcasting the handler.
    #[must_use]
    pub fn new(action: A) -> Self {
        let config = action.config();
        Self {
            action,
            config,
            state: RwLock::new(None),
            in_flight: Arc::new(AtomicU32::new(0)),
            idle_notify: Arc::new(Notify::new()),
        }
    }

    /// Webhook config cached at construction.
    ///
    /// Runtime code that wraps an action into a `WebhookTriggerAdapter`
    /// reads this and forwards it to
    /// `nebula_api::webhook::WebhookTransport::activate`. Tests can
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

#[async_trait]
impl<A> TriggerHandler for WebhookTriggerAdapter<A>
where
    A: WebhookAction + Send + Sync + 'static,
    A::State: Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
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
                    action = %self.action.metadata().base.key,
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
    }

    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError> {
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
    async fn handle_event(
        &self,
        event: TriggerEvent,
        ctx: &TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
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
        // the await on handle_request. Holding a parking_lot guard across .await
        // would be unsound (non-Send) and risk re-entry panic with start/stop.
        let state = match self.state.read().as_ref().cloned() {
            Some(s) => s,
            None => {
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
                ctx.health.record_error();
                return Err(ActionError::fatal(
                    "handle_event called before start or after stop — no state available",
                ));
            },
        };

        // H6 — cancellation-safe dispatch. If the trigger is being
        // shut down mid-request, send `503 Service Unavailable` to
        // the external caller and return a retryable error so the
        // runtime records it and the task exits cleanly. Otherwise
        // race the handler normally.
        let response = tokio::select! {
            biased;
            () = ctx.cancellation.cancelled() => {
                if let Some(tx) = response_tx {
                    let _ = tx.send(WebhookHttpResponse::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        Bytes::from_static(b"shutting down"),
                    ));
                }
                ctx.health.record_error();
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
                        ctx.health.record_error();
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
            TriggerEventOutcome::Emit(_) => ctx.health.record_success(1),
            TriggerEventOutcome::EmitMany(batch) => {
                ctx.health.record_success(batch.len() as u64);
            },
            TriggerEventOutcome::Skip => ctx.health.record_idle(),
        }

        Ok(outcome)
    }
}

impl<A: WebhookAction> fmt::Debug for WebhookTriggerAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookTriggerAdapter")
            .field("action", &self.action.metadata().base.key)
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
    ///     return Ok(TriggerEventOutcome::skip());
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
/// - `secret`  — shared HMAC key (typically from a credential)
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
///     return Ok(TriggerEventOutcome::skip());
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
