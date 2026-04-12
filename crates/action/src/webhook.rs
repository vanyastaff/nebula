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
//!
//! # Security
//!
//! **Never compare signatures with `==` or `str::eq`.** The byte-wise
//! short-circuit in `PartialEq` leaks the secret one prefix byte at a
//! time and is exploitable over the network. Always use the helpers in
//! this module.
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
    time::SystemTime,
};

use async_trait::async_trait;
use bytes::Bytes;
use hmac::{Hmac, KeyInit, Mac};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use parking_lot::RwLock;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::oneshot;

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
/// We allow 128 to leave headroom for providers that include many
/// tracing/forwarding headers, while preventing an O(n²) CPU DoS if an
/// action called `header()` in a loop against a huge attacker-supplied
/// header set.
pub const MAX_HEADER_COUNT: usize = 128;

// ── WebhookRequest ──────────────────────────────────────────────────────────

/// Incoming HTTP webhook request — the typed event carried inside a
/// [`TriggerEvent`](crate::trigger::TriggerEvent) for webhook triggers.
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
    /// this call inherits that DoS protection without an additional
    /// byte-size guard.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the body is not valid JSON.
    pub fn body_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
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
    /// in a [`TriggerEvent`](crate::trigger::TriggerEvent). The adapter
    /// sends a [`WebhookHttpResponse`] through the channel after calling
    /// [`WebhookAction::handle_request`].
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
    state: RwLock<Option<Arc<A::State>>>,
    in_flight: AtomicU32,
}

impl<A: WebhookAction> WebhookTriggerAdapter<A> {
    /// Wrap a typed webhook action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self {
            action,
            state: RwLock::new(None),
            in_flight: AtomicU32::new(0),
        }
    }
}

/// RAII guard that decrements the in-flight counter on drop.
struct InFlightGuard<'a>(&'a AtomicU32);

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
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
                    action = %self.action.metadata().key,
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
                // Wait for in-flight handle_event calls to complete before
                // calling on_deactivate. Each in-flight request holds its
                // own Arc clone, so on_deactivate sees the state after all
                // requests finish (not while they're mid-flight).
                while self.in_flight.load(Ordering::Acquire) > 0 {
                    tokio::task::yield_now().await;
                }
                let owned = Arc::unwrap_or_clone(arc_state);
                self.action.on_deactivate(owned, ctx).await
            }
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
            }
        };

        let response_tx = request.take_response_tx();

        self.in_flight.fetch_add(1, Ordering::AcqRel);
        let _guard = InFlightGuard(&self.in_flight);

        // Clone Arc under read lock; the guard drops at end of statement BEFORE
        // the await on handle_request. Holding a parking_lot guard across .await
        // would be unsound (non-Send) and risk re-entry panic with start/stop.
        let state = self.state.read().as_ref().cloned().ok_or_else(|| {
            ActionError::fatal("handle_event called before start — no state available")
        })?;

        let response = self.action.handle_request(&request, &state, ctx).await?;
        let (http_response, outcome) = response.into_parts();

        if let Some(tx) = response_tx {
            let _ = tx.send(http_response);
        }

        Ok(outcome)
    }
}

impl<A: WebhookAction> fmt::Debug for WebhookTriggerAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookTriggerAdapter")
            .field("action", &self.action.metadata().key)
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

    let Some(sig_header) = request.header(&name) else {
        return Ok(SignatureOutcome::Missing);
    };

    // Strip the common GitHub-style prefix. Other schemes that embed
    // metadata in the header (Stripe `t=…,v1=…`) are not handled here —
    // use `hmac_sha256_compute` + `verify_tag_constant_time` directly.
    let sig_hex = sig_header
        .strip_prefix("sha256=")
        .unwrap_or(sig_header)
        .trim();

    let Ok(expected) = hex::decode(sig_hex) else {
        return Ok(SignatureOutcome::Invalid);
    };

    // Reason: `Hmac::new_from_slice` returns `InvalidLength` only for
    // block-cipher MACs (CMAC etc.). For HMAC (RFC 2104) any key
    // length is accepted — oversize keys are hashed to block size,
    // undersize keys are zero-padded. Surfacing this as
    // `ActionError::Fatal` would poison callers with an impossible
    // error variant. The empty-secret guard above is the only length
    // check HMAC actually needs.
    #[allow(clippy::expect_used)]
    let mut mac =
        HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length (RFC 2104)");
    mac.update(request.body());

    Ok(match mac.verify_slice(&expected) {
        Ok(()) => SignatureOutcome::Valid,
        Err(_) => SignatureOutcome::Invalid,
    })
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
    #[allow(clippy::expect_used)]
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
