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

use axum::{Router, extract::DefaultBodyLimit, routing::post};
use nebula_action::{
    Clock, SystemClock, TriggerHandler, TriggerRuntimeContext, WebhookConfig,
    WebhookEndpointProvider,
};
use nebula_metrics::MetricsRegistry;
use url::Url;
use uuid::Uuid;

use super::dispatch::{slug_webhook_handler, webhook_handler};
use super::ratelimit::WebhookRateLimiter;
use super::{
    key::WebhookKey,
    provider::EndpointProviderImpl,
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
    pub(super) trigger_uuid: Uuid,
    pub(super) nonce: String,
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
    pub(super) inner: Arc<TransportInner>,
}

pub(super) struct TransportInner {
    pub(super) config: WebhookTransportConfig,
    pub(super) routing: RoutingMap,
    pub(super) rate_limiter: Option<WebhookRateLimiter>,
    /// Optional metrics registry. When `Some`, signature-failure
    /// outcomes increment [`NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`]
    /// per ADR-0022. `None` means the transport runs without emitting
    /// the counter — the enforcement behaviour is identical.
    pub(super) metrics: Option<Arc<MetricsRegistry>>,
    /// Time source for replay-window enforcement (M3.3 / ADR-0049).
    /// Production deployments use [`SystemClock`]; tests inject
    /// `MockClock` to drive deterministic timestamp scenarios.
    pub(super) clock: Arc<dyn Clock>,
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
