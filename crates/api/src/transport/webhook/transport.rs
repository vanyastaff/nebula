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
use nebula_storage_port::store::WebhookActivationStore;
use url::Url;
use uuid::Uuid;

use super::dispatch::webhook_handler;
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
    ///. `None` means the transport runs without emitting
    /// the counter — the enforcement behaviour is identical.
    pub(super) metrics: Option<Arc<MetricsRegistry>>,
    /// Time source for replay-window enforcement (webhook activation).
    /// Production deployments use [`SystemClock`]; tests inject
    /// `MockClock` to drive deterministic timestamp scenarios.
    pub(super) clock: Arc<dyn Clock>,
    /// Optional B-world port store (ADR-0096).
    ///
    /// When `Some`, `webhook_handler` resolves an incoming capability
    /// token to its durable row via `store.resolve_by_token(&hash)`,
    /// confirming the scope/workflow_id/mode tuple.  Dispatch still
    /// goes through the in-memory routing map — durable emitter install
    /// is deferred to U-D1.4b (next sub-slice).
    pub(super) activation_store: Option<Arc<dyn WebhookActivationStore>>,
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
    /// `NEBULA_WEBHOOK_SIGNATURE_FAILURES_TOTAL`.
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
                activation_store: None,
            }),
        }
    }

    /// Attach the B-world port store for durable token resolution.
    ///
    /// When set, the webhook dispatch handler will resolve incoming capability
    /// tokens via `store.resolve_by_token` in addition to the in-memory routing
    /// map lookup.
    ///
    /// This method returns a **new** `WebhookTransport` — the inner `Arc`
    /// is replaced (not mutated in place) so existing clones of the old
    /// transport do not observe the change.  Call this before distributing
    /// the transport to handlers.
    #[must_use = "builder methods must be chained or the result used"]
    pub fn with_activation_store(self, store: Arc<dyn WebhookActivationStore>) -> Self {
        // Destructure inner to rebuild with the store attached.
        // SAFETY: Arc::try_unwrap succeeds only if there are no other Arc
        // handles — this builder is expected at construction time before
        // the transport is cloned into handlers.  If other handles exist
        // we fall back to a clone of all fields.
        let inner = match Arc::try_unwrap(self.inner) {
            Ok(mut i) => {
                i.activation_store = Some(store);
                Arc::new(i)
            },
            Err(arc) => {
                // Already shared — build a new TransportInner by copying
                // fields from the existing one.
                let rate_limiter = arc
                    .config
                    .rate_limit_per_minute
                    .map(WebhookRateLimiter::new);
                Arc::new(TransportInner {
                    config: arc.config.clone(),
                    routing: RoutingMap::new(),
                    rate_limiter,
                    metrics: arc.metrics.clone(),
                    clock: arc.clock.clone(),
                    activation_store: Some(store),
                })
            },
        };
        Self { inner }
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
    ///  the caller reads it from the action (typically via
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

    /// Total active registrations. Used by `/healthz` reporters.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.inner.routing.len()
    }

    /// Build the axum router that dispatches incoming webhook
    /// requests to registered triggers.
    ///
    /// Mounts the programmatic URL shape:
    /// `POST {path_prefix}/{trigger_uuid}/{nonce}` — minted by [`Self::activate`].
    ///
    /// Slug-routed activations were retired in ADR-0096 commit 3; the
    /// routing map is now programmatic-only.
    pub fn router(&self) -> Router {
        let programmatic = format!(
            "{prefix}/{{trigger_uuid}}/{{nonce}}",
            prefix = self.inner.config.path_prefix,
        );
        Router::new()
            .route(&programmatic, post(webhook_handler))
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
/// 122 bits of entropy (`Uuid::new_v4` fixes 6 of its 128 bits for
/// version/variant) — above the W3C ≥120-bit capability-URL bar and
/// enough to make nonce collisions/guessing infeasible over the lifetime
/// of any Nebula deployment. Uses `Uuid::new_v4` because uuid is already
/// pulled in.
fn generate_nonce() -> String {
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();
    let mut out = String::with_capacity(32);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ── Mint-persist wrapper (ADR-0096 commit 2b) ────────────────────────────────

/// Error returned by [`activate_and_persist`].
#[derive(Debug, thiserror::Error)]
pub enum ActivateAndPersistError {
    /// The in-memory activation failed (routing-map duplicate collision).
    #[error("activation failed: {0}")]
    Activation(#[from] ActivationError),
    /// The durability upsert to the B-world port store failed.
    ///
    /// The in-memory routing entry was already inserted at this point.
    /// The handle is still usable for the current process lifetime; the
    /// token simply will not survive a restart.  Callers may choose to
    /// deactivate and surface an error to the API layer.
    #[error("failed to persist activation token: {0}")]
    Storage(#[from] nebula_storage_port::StorageError),
}

/// Parameters for [`activate_and_persist`].
///
/// Bundles the per-activation inputs into one struct so the function signature
/// stays within `clippy::too-many-arguments` limits.
pub struct PersistParams {
    /// The webhook action handler that will service inbound requests.
    pub handler: Arc<dyn TriggerHandler>,
    /// Webhook-specific action configuration (method filter, path prefix, etc.).
    pub action_config: WebhookConfig,
    /// Context template forwarded to dispatch on each incoming request.
    pub ctx_template: TriggerRuntimeContext,
    /// Trigger identity in the B-world store (`triggers.trigger_id`).
    pub trigger_id: String,
    /// Tenant scope the activation belongs to; used as the store partition key.
    pub scope: nebula_storage_port::Scope,
    /// Optional workflow associated with this activation.
    pub workflow_id: Option<String>,
    /// Delivery mode (fire-and-forget vs durable-at-least-once).
    pub mode: nebula_storage_port::dto::WebhookMode,
}

/// Mint-persist wrapper for programmatic webhook activations.
///
/// Keeps [`WebhookTransport`] in-memory-pure: the transport itself only sees
/// the routing map; this wrapper adds the durability layer on top.
///
/// # Contract
///
/// 1. Calls `transport.activate(params.handler, params.action_config,
///    params.ctx_template)` to mint `(trigger_uuid, nonce)` and insert the
///    in-memory routing entry.
/// 2. Computes `token_hash = SHA-256(nonce)` at the API edge.
/// 3. Upserts `WebhookActivationRecord { trigger_id, scope, slug, active: true,
///    workflow_id, mode, token_hash }` to `store`.
/// 4. Returns the plaintext [`ActivationHandle`] (endpoint URL + context) to
///    the caller **exactly once**.  The plaintext nonce is not persisted.
///
/// # Errors
///
/// Returns [`ActivateAndPersistError::Activation`] if the transport's in-memory
/// activation fails (effectively unreachable — duplicate nonce collision).
///
/// Returns [`ActivateAndPersistError::Storage`] if the port store upsert fails.
/// In this case the in-memory entry exists but the token is not durable.  The
/// caller must decide whether to surface an error and deactivate.
///
/// # Security
///
/// The plaintext `nonce` (the bearer token embedded in the capability URL)
/// never leaves this function; only its SHA-256 hash reaches the store.
pub async fn activate_and_persist(
    transport: &WebhookTransport,
    store: &dyn WebhookActivationStore,
    params: PersistParams,
) -> Result<ActivationHandle, ActivateAndPersistError> {
    let PersistParams {
        handler,
        action_config,
        ctx_template,
        trigger_id,
        scope,
        workflow_id,
        mode,
    } = params;

    // Step 1: in-memory activation (mints uuid + nonce, inserts routing entry).
    let handle = transport.activate(handler, action_config, ctx_template)?;

    // Step 2: hash at the API edge — nonce never leaves this stack frame.
    let hash = super::token::token_hash(&handle.nonce);

    // Step 3: build the B-world record.
    //
    // `slug` is set to the trigger UUID string so the port store has a unique,
    // human-readable identifier for the activation.  This field is used only
    // for the slug-keyed `resolve` path (not exercised for programmatic
    // activations); its value does not affect routing or token resolution.
    let mut record = nebula_storage_port::dto::WebhookActivationRecord::new(
        trigger_id,
        scope.clone(),
        handle.trigger_uuid.to_string(),
        true,
    );
    record.workflow_id = workflow_id;
    record.mode = mode;
    record.token_hash = hash;

    // Step 4: upsert to the port store.
    store.upsert(&scope, record).await?;

    tracing::debug!(
        target: "nebula::api::webhook::transport",
        trigger_uuid = %handle.trigger_uuid,
        // nonce deliberately excluded from this span
        "programmatic activation persisted to port store"
    );

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_action::{TriggerContext, TriggerHandler, WebhookConfig};
    use nebula_core::BaseContext;
    use nebula_storage::inmem::InMemoryWebhookActivationStore;
    use nebula_storage_port::Scope;
    use nebula_storage_port::dto::WebhookMode;
    use nebula_storage_port::store::WebhookActivationStore;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::transport::webhook::key::WebhookKey;
    use crate::transport::webhook::token::token_hash;

    // Minimal no-op TriggerHandler for tests.
    struct Noop {
        meta: nebula_action::ActionMetadata,
    }

    #[async_trait::async_trait]
    impl TriggerHandler for Noop {
        fn metadata(&self) -> &nebula_action::ActionMetadata {
            &self.meta
        }
        async fn start(&self, _ctx: &dyn TriggerContext) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }
        async fn stop(&self, _ctx: &dyn TriggerContext) -> Result<(), nebula_action::ActionError> {
            Ok(())
        }
    }

    fn noop_handler() -> Arc<dyn TriggerHandler> {
        Arc::new(Noop {
            meta: nebula_action::ActionMetadata::new(
                nebula_core::action_key!("test.transport.noop"),
                "Noop",
                "mint-persist test",
            ),
        })
    }

    fn ctx_template() -> TriggerRuntimeContext {
        TriggerRuntimeContext::new(
            Arc::new(
                BaseContext::builder()
                    .cancellation(CancellationToken::new())
                    .build(),
            ),
            nebula_core::WorkflowId::new(),
            nebula_core::node_key!("test"),
        )
    }

    fn test_scope() -> Scope {
        Scope::new("test-org", "test-ws")
    }

    /// Mint-persist round-trip:
    ///
    /// - `activate_and_persist` mints the token and upserts the hashed record.
    /// - `resolve_by_token(sha256(plaintext))` returns the row with matching
    ///   scope, trigger_id, and workflow_id.
    /// - The port store row does NOT contain the plaintext token.
    #[tokio::test]
    async fn mint_persist_round_trip() {
        let transport = WebhookTransport::new(WebhookTransportConfig::default());
        let store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let trigger_id = "trigger-abc-123";
        let workflow_id = Some("wf-xyz-456".to_string());
        let scope = test_scope();

        let handle = activate_and_persist(
            &transport,
            store.as_ref(),
            PersistParams {
                handler: noop_handler(),
                action_config: WebhookConfig::default(),
                ctx_template: ctx_template(),
                trigger_id: trigger_id.to_string(),
                scope: scope.clone(),
                workflow_id: workflow_id.clone(),
                mode: WebhookMode::Test,
            },
        )
        .await
        .expect("activate_and_persist must succeed");

        // Compute the hash as the consumer would.
        let hash = token_hash(&handle.nonce);

        // resolve_by_token must find the row.
        let row = store
            .resolve_by_token(&hash)
            .await
            .expect("storage must not error")
            .expect("row must be found by token hash");

        assert_eq!(row.trigger_id, trigger_id, "trigger_id must round-trip");
        assert_eq!(row.scope, scope, "scope must round-trip");
        assert_eq!(row.workflow_id, workflow_id, "workflow_id must round-trip");
        assert_eq!(row.mode, WebhookMode::Test, "mode must round-trip");

        // The stored token_hash matches the hash we computed from the plaintext.
        assert_eq!(row.token_hash, hash, "stored hash must equal sha256(nonce)");

        // The plaintext nonce is NOT stored in any field of the DTO.
        // Verify via the serialised form — serde JSON must not contain the nonce.
        let json = serde_json::to_string(&row).expect("DTO must be serialisable");
        assert!(
            !json.contains(&handle.nonce),
            "serialised DTO must not contain the plaintext nonce. Got: {json}"
        );
    }

    /// The in-memory routing map entry is present after `activate_and_persist`
    /// so the transport can dispatch the activation immediately without a
    /// storage read.
    #[tokio::test]
    async fn activate_and_persist_populates_routing_map() {
        let transport = WebhookTransport::new(WebhookTransportConfig::default());
        let store: Arc<dyn WebhookActivationStore> =
            Arc::new(InMemoryWebhookActivationStore::new());

        let handle = activate_and_persist(
            &transport,
            store.as_ref(),
            PersistParams {
                handler: noop_handler(),
                action_config: WebhookConfig::default(),
                ctx_template: ctx_template(),
                trigger_id: "trigger-routing-test".to_string(),
                scope: test_scope(),
                workflow_id: None,
                mode: WebhookMode::Test,
            },
        )
        .await
        .expect("activate_and_persist must succeed");

        let key = WebhookKey::programmatic(handle.trigger_uuid, handle.nonce);
        assert!(
            transport.inner.routing.lookup(&key).is_some(),
            "in-memory routing map must have the entry after activate_and_persist"
        );
    }
}
