//! Slug-routed webhook dispatcher.
//!
//! Bridges `POST /api/v1/hooks/{org}/{ws}/{trigger_slug}` to the
//! engine layer. Each registered trigger lives in an in-memory
//! [`DashMap`] keyed by [`TriggerCoordinates`]; the dispatcher looks
//! up the entry, validates per-trigger auth, builds a typed
//! [`WebhookRequest`], wraps it in a [`TriggerEvent`], and hands it to
//! the registered [`WebhookTriggerSink`].
//!
//! ## Why a separate dispatcher?
//!
//! The mature [`crate::services::webhook::WebhookTransport`] is keyed
//! on `(uuid, nonce)` and built around the typed
//! [`nebula_action::WebhookAction`] surface. That layout is right for
//! programmatic activation by `nebula-action` runtime registrations
//! but not for human-facing slug URLs that map to operator-configured
//! triggers in the storage layer. Building the slug-routed dispatcher
//! as a sibling type keeps the typed-action transport contract
//! untouched.
//!
//! ## Invariants
//!
//! - Registrations are unique per `(org, ws, slug)`. A second `register` for the same coordinates
//!   returns [`RegisterError::AlreadyRegistered`] without overwriting.
//! - `dispatch` returns 202 semantics: the event is enqueued through the sink, and the HTTP layer
//!   must not synthesise a response from the trigger payload. Engine-side processing is async by
//!   design.
//! - Auth is checked **before** the sink is touched, so a 401 path never increments the engine's
//!   accepted-event count.
//! - The dispatcher emits a `tracing` span for every dispatch. The span carries the slug tuple, the
//!   request body size, and the typed error (when one occurs) per the
//!   `feedback_observability_as_completion.md` DoD.

use std::sync::Arc;

use axum::{
    body::Bytes,
    http::{HeaderMap, Method, Uri},
};
use dashmap::DashMap;
use nebula_action::{TriggerEvent, WebhookRequest};
use thiserror::Error;
use tracing::{Instrument, debug, debug_span, warn};

use super::{
    auth::{WebhookAuthConfig, validate as validate_auth},
    error::{TriggerCoordinates, WebhookDispatchError, WebhookEnqueueError},
    sink::{NoopSink, WebhookTriggerSink},
};

/// Single registered webhook trigger.
///
/// Holds the per-trigger auth policy and a per-trigger sink. Each
/// registration can carry its own sink so a multi-tenant deployment
/// can route different triggers to different engine queues — but a
/// shared sink is the common case (one engine per dispatcher).
#[derive(Clone)]
pub struct TriggerRegistration {
    /// Per-trigger authentication policy.
    pub auth: WebhookAuthConfig,
    /// Engine-facing sink. `None` falls through to the dispatcher's
    /// default sink, set by [`WebhookDispatcher::with_default_sink`].
    pub sink: Option<Arc<dyn WebhookTriggerSink>>,
}

impl TriggerRegistration {
    /// Build a registration that uses the dispatcher's default sink.
    #[must_use]
    pub fn new(auth: WebhookAuthConfig) -> Self {
        Self { auth, sink: None }
    }

    /// Build a registration with a per-trigger sink override.
    #[must_use]
    pub fn with_sink(auth: WebhookAuthConfig, sink: Arc<dyn WebhookTriggerSink>) -> Self {
        Self {
            auth,
            sink: Some(sink),
        }
    }
}

impl std::fmt::Debug for TriggerRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerRegistration")
            .field("auth", &"<elided>")
            .field("has_per_trigger_sink", &self.sink.is_some())
            .finish_non_exhaustive()
    }
}

/// In-memory slug-keyed dispatcher.
#[derive(Debug)]
pub struct WebhookDispatcher {
    registrations: DashMap<TriggerCoordinates, TriggerRegistration>,
    default_sink: Arc<dyn WebhookTriggerSink>,
}

/// Errors raised by [`WebhookDispatcher::register`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegisterError {
    /// A registration already exists for the same `(org, ws, slug)`
    /// tuple. The caller should `unregister` first or treat as a bug.
    #[error("trigger already registered for {0:?}")]
    AlreadyRegistered(TriggerCoordinates),
}

impl Default for WebhookDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookDispatcher {
    /// Build a new dispatcher with a [`NoopSink`] as the default.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registrations: DashMap::new(),
            default_sink: Arc::new(NoopSink::new()),
        }
    }

    /// Override the default sink. Triggers registered without a
    /// per-trigger sink delegate here.
    #[must_use]
    pub fn with_default_sink(mut self, sink: Arc<dyn WebhookTriggerSink>) -> Self {
        self.default_sink = sink;
        self
    }

    /// Register a trigger.
    ///
    /// # Errors
    ///
    /// Returns [`RegisterError::AlreadyRegistered`] if a registration
    /// already exists for the same coordinates.
    pub fn register(
        &self,
        coordinates: TriggerCoordinates,
        registration: TriggerRegistration,
    ) -> Result<(), RegisterError> {
        match self.registrations.entry(coordinates.clone()) {
            dashmap::Entry::Occupied(_) => Err(RegisterError::AlreadyRegistered(coordinates)),
            dashmap::Entry::Vacant(v) => {
                v.insert(registration);
                Ok(())
            },
        }
    }

    /// Unregister a trigger. Returns `true` when an entry was removed,
    /// `false` when no registration existed.
    pub fn unregister(&self, coordinates: &TriggerCoordinates) -> bool {
        self.registrations.remove(coordinates).is_some()
    }

    /// Number of currently-registered triggers. Mostly for tests and
    /// observability dashboards.
    #[must_use]
    pub fn registration_count(&self) -> usize {
        self.registrations.len()
    }

    /// Dispatch an inbound webhook request.
    ///
    /// Steps, in order:
    /// 1. Look up registration → [`WebhookDispatchError::NotFound`].
    /// 2. Build a typed [`WebhookRequest`] (caps enforced).
    /// 3. Validate per-trigger auth → [`WebhookDispatchError::Auth`].
    /// 4. Wrap in [`TriggerEvent`] and enqueue through the sink →
    ///    [`WebhookDispatchError::Enqueue`].
    ///
    /// On success the dispatcher returns `Ok(())` and the HTTP layer
    /// should reply with `202 Accepted`. The sink decides whether the
    /// event is consumed synchronously or buffered.
    pub async fn dispatch(
        &self,
        coordinates: TriggerCoordinates,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<(), WebhookDispatchError> {
        let span = debug_span!(
            "webhook.dispatch",
            org = %coordinates.org,
            workspace = %coordinates.workspace,
            trigger = %coordinates.trigger,
            body_bytes = body.len(),
        );
        async move {
            let registration = self
                .registrations
                .get(&coordinates)
                .map(|r| r.clone())
                .ok_or_else(|| WebhookDispatchError::NotFound {
                    trigger: coordinates.clone(),
                })?;

            let path = uri.path().to_string();
            let query = uri.query().map(str::to_string);
            let request =
                WebhookRequest::try_new(method, path, query, headers, body).map_err(|err| {
                    debug!(error = %err, "webhook request construction failed");
                    // The slug-routed handler maps malformed requests
                    // to 401 only when auth fails; a construction
                    // failure (header count, etc.) is treated as a
                    // missing-signature outcome — fail-closed.
                    WebhookDispatchError::Auth(super::error::WebhookAuthError::SignatureInvalid)
                })?;

            validate_auth(&registration.auth, &request)?;

            let event = TriggerEvent::new(None, request);
            let sink = registration
                .sink
                .clone()
                .unwrap_or_else(|| Arc::clone(&self.default_sink));

            sink.enqueue(&coordinates, event).await.map_err(|err| {
                warn!(error = %err, "webhook trigger sink rejected event");
                WebhookDispatchError::Enqueue(map_enqueue_error(err, &coordinates))
            })?;

            Ok::<(), WebhookDispatchError>(())
        }
        .instrument(span)
        .await
    }
}

/// Re-stamp the coordinates on a sink-returned enqueue error so the
/// HTTP layer can surface the failing trigger even if the sink built
/// the error with placeholder coordinates (some impls do).
fn map_enqueue_error(
    err: WebhookEnqueueError,
    coordinates: &TriggerCoordinates,
) -> WebhookEnqueueError {
    match err {
        WebhookEnqueueError::SinkUnavailable { .. } => WebhookEnqueueError::SinkUnavailable {
            trigger: coordinates.clone(),
        },
        WebhookEnqueueError::SinkBackpressure { .. } => WebhookEnqueueError::SinkBackpressure {
            trigger: coordinates.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, Method, Uri};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    use super::*;
    use crate::webhook::sink::{DispatchedEvent, MpscSink};

    type HmacSha256 = Hmac<Sha256>;

    fn coords() -> TriggerCoordinates {
        TriggerCoordinates::new("acme", "main", "github")
    }

    fn other_coords() -> TriggerCoordinates {
        TriggerCoordinates::new("acme", "main", "stripe")
    }

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn make_uri() -> Uri {
        "/api/v1/hooks/acme/main/github".parse().unwrap()
    }

    async fn install(
        dispatcher: &WebhookDispatcher,
        coordinates: TriggerCoordinates,
        registration: TriggerRegistration,
    ) {
        dispatcher.register(coordinates, registration).unwrap();
    }

    #[tokio::test]
    async fn unregistered_path_returns_not_found() {
        let dispatcher = WebhookDispatcher::new();
        let err = dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, WebhookDispatchError::NotFound { .. }));
    }

    #[tokio::test]
    async fn dispatch_passes_event_to_default_sink() {
        let sink = Arc::new(MpscSink::new(4));
        let mut rx = sink.take_receiver().await.unwrap();
        let dispatcher = WebhookDispatcher::new().with_default_sink(sink);
        install(
            &dispatcher,
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::None),
        )
        .await;

        dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::from_static(br#"{"hello":"world"}"#),
            )
            .await
            .unwrap();

        let dispatched: DispatchedEvent = rx.recv().await.expect("event delivered");
        assert_eq!(dispatched.coordinates, coords());
        // Body roundtrips through the WebhookRequest payload.
        let (_, _, request) = dispatched
            .event
            .downcast::<WebhookRequest>()
            .expect("payload is WebhookRequest");
        assert_eq!(request.body(), b"{\"hello\":\"world\"}");
    }

    #[tokio::test]
    async fn dispatch_routes_per_trigger_sink_independently() {
        let global_sink = Arc::new(MpscSink::new(4));
        let mut global_rx = global_sink.take_receiver().await.unwrap();
        let per_trigger_sink = Arc::new(MpscSink::new(4));
        let mut per_trigger_rx = per_trigger_sink.take_receiver().await.unwrap();

        let dispatcher = WebhookDispatcher::new().with_default_sink(global_sink);
        install(
            &dispatcher,
            coords(),
            TriggerRegistration::with_sink(WebhookAuthConfig::None, per_trigger_sink),
        )
        .await;
        install(
            &dispatcher,
            other_coords(),
            TriggerRegistration::new(WebhookAuthConfig::None),
        )
        .await;

        dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();
        dispatcher
            .dispatch(
                other_coords(),
                Method::POST,
                "/api/v1/hooks/acme/main/stripe".parse().unwrap(),
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap();

        let per_trigger = per_trigger_rx.recv().await.unwrap();
        assert_eq!(per_trigger.coordinates, coords());

        let fallback = global_rx.recv().await.unwrap();
        assert_eq!(fallback.coordinates, other_coords());
    }

    #[tokio::test]
    async fn dispatch_fails_when_signature_missing() {
        let secret: Arc<[u8]> = Arc::<[u8]>::from(b"shh".as_slice());
        let dispatcher = WebhookDispatcher::new();
        install(
            &dispatcher,
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::hmac_sha256(secret)),
        )
        .await;

        let err = dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            WebhookDispatchError::Auth(super::super::error::WebhookAuthError::SignatureMissing)
        ));
    }

    #[tokio::test]
    async fn dispatch_passes_when_signature_valid() {
        let secret: Arc<[u8]> = Arc::<[u8]>::from(b"shh".as_slice());
        let body = br#"{"event":"push"}"#.to_vec();
        let mut headers = HeaderMap::new();
        headers.insert(
            crate::webhook::auth::DEFAULT_SIGNATURE_HEADER,
            HeaderValue::from_str(&sign(&secret, &body)).unwrap(),
        );
        let sink = Arc::new(MpscSink::new(1));
        let mut rx = sink.take_receiver().await.unwrap();
        let dispatcher = WebhookDispatcher::new().with_default_sink(sink);
        install(
            &dispatcher,
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::hmac_sha256(secret)),
        )
        .await;

        dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                headers,
                Bytes::from(body),
            )
            .await
            .unwrap();

        let _delivered = rx.recv().await.expect("hmac-validated event delivered");
    }

    #[tokio::test]
    async fn dispatch_returns_enqueue_error_when_sink_closed() {
        let sink = Arc::new(MpscSink::new(1));
        let rx = sink.take_receiver().await.unwrap();
        drop(rx); // close the channel
        let dispatcher = WebhookDispatcher::new().with_default_sink(sink);
        install(
            &dispatcher,
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::None),
        )
        .await;

        let err = dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            WebhookDispatchError::Enqueue(WebhookEnqueueError::SinkUnavailable { .. })
        ));
    }

    #[tokio::test]
    async fn register_rejects_duplicate() {
        let dispatcher = WebhookDispatcher::new();
        dispatcher
            .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
            .unwrap();
        let err = dispatcher
            .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
            .unwrap_err();
        assert_eq!(err, RegisterError::AlreadyRegistered(coords()));
    }

    #[tokio::test]
    async fn unregister_removes_route() {
        let dispatcher = WebhookDispatcher::new();
        dispatcher
            .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
            .unwrap();
        assert_eq!(dispatcher.registration_count(), 1);
        assert!(dispatcher.unregister(&coords()));
        assert_eq!(dispatcher.registration_count(), 0);
        assert!(!dispatcher.unregister(&coords()));
    }

    #[tokio::test]
    async fn dispatch_after_unregister_returns_not_found() {
        let dispatcher = WebhookDispatcher::new();
        dispatcher
            .register(coords(), TriggerRegistration::new(WebhookAuthConfig::None))
            .unwrap();
        dispatcher.unregister(&coords());
        let err = dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, WebhookDispatchError::NotFound { .. }));
    }

    #[tokio::test]
    async fn dispatch_does_not_consume_sink_capacity_on_auth_failure() {
        let sink: Arc<dyn WebhookTriggerSink> = Arc::new(MpscSink::new(1));
        // We deliberately leak the receiver into a holder so the
        // channel stays open for the post-rejection capacity probe.
        let mpsc_sink = Arc::clone(&sink);
        // Downcast not available on dyn; rely on the fact that the
        // channel has 1 slot.
        let dispatcher = WebhookDispatcher::new().with_default_sink(Arc::clone(&sink));
        install(
            &dispatcher,
            coords(),
            TriggerRegistration::new(WebhookAuthConfig::hmac_sha256(Arc::<[u8]>::from(
                b"shh".as_slice(),
            ))),
        )
        .await;

        // Bad signature → auth failure → sink must NOT be touched.
        let _ = dispatcher
            .dispatch(
                coords(),
                Method::POST,
                make_uri(),
                HeaderMap::new(),
                Bytes::from_static(b"{}"),
            )
            .await
            .unwrap_err();

        // Channel capacity is still 1 — push one event ourselves to
        // prove the auth gate fired before enqueue.
        mpsc_sink
            .enqueue(&coords(), TriggerEvent::new(None, 1_u32))
            .await
            .expect("capacity preserved by auth-rejection short-circuit");
    }
}
