//! `TriggerLifecycleEvent` and the transport subscriber (M3.3 / ADR-0049 — E2).
//!
//! Operator changes to slug-routed webhook activations (create /
//! update / delete) are published as
//! [`TriggerLifecycleEvent`]s on a dedicated [`EventBus`]. The
//! transport subscribes and reflects the change in its slug map
//! without reloading the entire registry.
//!
//! # Producer scope
//!
//! E2 ships the **consumer** — the transport-side subscriber that
//! reapplies events. The producer side (storage CRUD callsites that
//! `emit()` on the bus) is intentionally out of scope for M3.3 per
//! ADR-0049 §"Out of scope". Until producers wire in, the bus is
//! observed only by the admin reload endpoint and by tests.
//!
//! # Why an event bus, not direct calls
//!
//! - Multi-subscriber: future consumers (audit, dashboards) bind to
//!   the same bus without changing producer code.
//! - Decoupling: the storage CRUD side does not know about the API
//!   transport.
//! - Replayability: subscribers see lag via
//!   [`nebula_eventbus::Subscriber::recv`] when they fall behind.

use std::sync::Arc;

#[cfg(test)]
use async_trait::async_trait;
use nebula_action::{
    BuiltWebhookHandler, FactoryError, TriggerHandler, TriggerRuntimeContext,
    webhook::factory::WebhookActivationSpec as ActionWebhookActivationSpec,
};
use nebula_engine::ActionRegistry;
use nebula_eventbus::EventBus;
use nebula_storage::rows::{
    WebhookActivationCoords, WebhookActivationRecord,
    WebhookActivationSpec as StorageWebhookActivationSpec,
};
use thiserror::Error;
use tokio::task::JoinHandle;

use super::{
    bootstrap::{SecretResolutionError, WebhookContextFactory, WebhookSecretResolver},
    key::TriggerCoordinates,
    transport::{ActivationError, WebhookTransport},
};

/// Operator-driven mutations to slug-routed webhook activations.
///
/// Each variant carries the storage-shaped record so subscribers do
/// not have to round-trip through the database to reapply the
/// change.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TriggerLifecycleEvent {
    /// A new operator-configured activation came online.
    Created(WebhookActivationRecord),
    /// An existing activation's spec changed. The transport
    /// unregisters the previous slug and registers the new handler.
    Updated(WebhookActivationRecord),
    /// An activation was archived or deleted. The transport drops
    /// the slug entry.
    Deleted(WebhookActivationCoords),
}

/// `EventBus` typed for [`TriggerLifecycleEvent`].
pub type TriggerLifecycleEventBus = EventBus<TriggerLifecycleEvent>;

/// Failure modes for the lifecycle subscriber.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LifecycleApplyError {
    /// Could not resolve the credential reference to raw secret bytes.
    #[error("failed to resolve secret '{secret_id}': {source}")]
    SecretResolution {
        /// Storage credential identifier.
        secret_id: String,
        /// Underlying cause.
        #[source]
        source: SecretResolutionError,
    },
    /// No factory registered for the spec's `action_kind`.
    #[error("no factory registered for kind '{0}'")]
    UnknownProvider(String),
    /// Factory rejected the spec.
    #[error("factory build failed for kind '{kind}': {source}")]
    Factory {
        /// Provider kind reported by the factory.
        kind: String,
        /// Underlying cause.
        #[source]
        source: FactoryError,
    },
    /// Two events resolved to the same slug coordinates without an
    /// intervening `Deleted`. Should not happen in production but
    /// the subscriber surfaces it instead of silently overwriting.
    #[error("duplicate slug registration for {coords:?}")]
    DuplicateRegistration {
        /// Slug coordinates already registered.
        coords: TriggerCoordinates,
    },
}

/// Transport subscriber bound to a [`TriggerLifecycleEventBus`].
///
/// `spawn` returns a [`JoinHandle`] running the subscribe loop on a
/// detached tokio task. Drop the handle to abort, or `await` it for
/// graceful shutdown after closing the bus.
pub struct TriggerLifecycleSubscriber {
    bus: Arc<TriggerLifecycleEventBus>,
    transport: WebhookTransport,
    registry: Arc<ActionRegistry>,
    secrets: Arc<dyn WebhookSecretResolver>,
    ctx_factory: Arc<dyn WebhookContextFactory>,
}

impl TriggerLifecycleSubscriber {
    /// Construct a subscriber. Caller still has to invoke
    /// [`Self::spawn`] to start consuming events.
    #[must_use]
    pub fn new(
        bus: Arc<TriggerLifecycleEventBus>,
        transport: WebhookTransport,
        registry: Arc<ActionRegistry>,
        secrets: Arc<dyn WebhookSecretResolver>,
        ctx_factory: Arc<dyn WebhookContextFactory>,
    ) -> Self {
        Self {
            bus,
            transport,
            registry,
            secrets,
            ctx_factory,
        }
    }

    /// Spawn the subscriber loop on a tokio task. The task runs
    /// until the bus is dropped or the loop encounters a non-recoverable
    /// error — per-event failures are logged and the loop continues.
    ///
    /// The bus subscription is created **before** the spawned task
    /// starts so events emitted immediately after this call are
    /// guaranteed to be observed.
    #[must_use = "the JoinHandle controls task lifetime"]
    pub fn spawn(self) -> JoinHandle<()> {
        let sub = self.bus.subscribe();
        tokio::spawn(self.run(sub))
    }

    async fn run(self, mut sub: nebula_eventbus::Subscriber<TriggerLifecycleEvent>) {
        tracing::debug!(
            target: "nebula::api::webhook::lifecycle",
            "trigger lifecycle subscriber started"
        );
        while let Some(event) = sub.recv().await {
            self.apply_event(&event).await;
        }
        tracing::debug!(
            target: "nebula::api::webhook::lifecycle",
            "trigger lifecycle subscriber exiting (bus closed)"
        );
    }

    async fn apply_event(&self, event: &TriggerLifecycleEvent) {
        match event {
            TriggerLifecycleEvent::Created(record) => {
                if let Err(err) = self.apply_register(record).await {
                    tracing::warn!(
                        target: "nebula::api::webhook::lifecycle",
                        error = %err,
                        org = %record.coords.org_slug,
                        workspace = %record.coords.workspace_slug,
                        trigger_slug = %record.coords.trigger_slug,
                        event_kind = "created",
                        "failed to apply trigger lifecycle event"
                    );
                } else {
                    tracing::info!(
                        target: "nebula::api::webhook::lifecycle",
                        event_kind = "created",
                        org = %record.coords.org_slug,
                        workspace = %record.coords.workspace_slug,
                        trigger_slug = %record.coords.trigger_slug,
                        "applied trigger lifecycle event"
                    );
                }
            },
            TriggerLifecycleEvent::Updated(record) => {
                let coords = TriggerCoordinates::new(
                    &record.coords.org_slug,
                    &record.coords.workspace_slug,
                    &record.coords.trigger_slug,
                );
                self.transport.unregister_slug(&coords);
                if let Err(err) = self.apply_register(record).await {
                    tracing::warn!(
                        target: "nebula::api::webhook::lifecycle",
                        error = %err,
                        org = %record.coords.org_slug,
                        workspace = %record.coords.workspace_slug,
                        trigger_slug = %record.coords.trigger_slug,
                        event_kind = "updated",
                        "failed to apply trigger lifecycle event"
                    );
                } else {
                    tracing::info!(
                        target: "nebula::api::webhook::lifecycle",
                        event_kind = "updated",
                        org = %record.coords.org_slug,
                        workspace = %record.coords.workspace_slug,
                        trigger_slug = %record.coords.trigger_slug,
                        "applied trigger lifecycle event"
                    );
                }
            },
            TriggerLifecycleEvent::Deleted(coords) => {
                let key = TriggerCoordinates::new(
                    &coords.org_slug,
                    &coords.workspace_slug,
                    &coords.trigger_slug,
                );
                let removed = self.transport.unregister_slug(&key);
                tracing::info!(
                    target: "nebula::api::webhook::lifecycle",
                    event_kind = "deleted",
                    org = %coords.org_slug,
                    workspace = %coords.workspace_slug,
                    trigger_slug = %coords.trigger_slug,
                    found_in_map = removed,
                    "applied trigger lifecycle event"
                );
            },
        }
    }

    async fn apply_register(
        &self,
        record: &WebhookActivationRecord,
    ) -> Result<(), LifecycleApplyError> {
        let factory = self
            .registry
            .lookup_webhook_factory(&record.spec.action_kind)
            .ok_or_else(|| LifecycleApplyError::UnknownProvider(record.spec.action_kind.clone()))?;

        let secret = self
            .secrets
            .resolve(&record.spec.secret_id)
            .await
            .map_err(|source| LifecycleApplyError::SecretResolution {
                secret_id: record.spec.secret_id.clone(),
                source,
            })?;
        let action_spec = into_action_spec(&record.spec, secret);

        let BuiltWebhookHandler { handler, config } =
            factory
                .build(&action_spec)
                .map_err(|source| LifecycleApplyError::Factory {
                    kind: record.spec.action_kind.clone(),
                    source,
                })?;

        let coords = TriggerCoordinates::new(
            &record.coords.org_slug,
            &record.coords.workspace_slug,
            &record.coords.trigger_slug,
        );
        let ctx = self.ctx_factory.build(record);

        register(&self.transport, coords, handler, config, ctx)
    }
}

fn register(
    transport: &WebhookTransport,
    coords: TriggerCoordinates,
    handler: Arc<dyn TriggerHandler>,
    config: nebula_action::WebhookConfig,
    ctx: TriggerRuntimeContext,
) -> Result<(), LifecycleApplyError> {
    transport
        .activate_slug(coords.clone(), handler, config, ctx)
        .map_err(|err| match err {
            ActivationError::DuplicateRegistration => {
                LifecycleApplyError::DuplicateRegistration { coords }
            },
            other => LifecycleApplyError::Factory {
                kind: "transport".to_owned(),
                source: FactoryError::InvalidSpec {
                    kind: "transport",
                    reason: other.to_string(),
                },
            },
        })
}

fn into_action_spec(
    storage: &StorageWebhookActivationSpec,
    secret: Vec<u8>,
) -> ActionWebhookActivationSpec {
    let mut spec = ActionWebhookActivationSpec::new(storage.action_kind.clone(), secret);
    if let Some(secs) = storage.replay_window_secs {
        spec = spec.with_replay_window_secs(secs);
    }
    if let Some(header) = storage.timestamp_header.as_ref() {
        spec = spec.with_timestamp_header(header.clone());
    }
    if let Some(config) = storage.provider_config.clone() {
        spec = spec.with_provider_config(config);
    }
    if let Some(rpm) = storage.rate_limit_per_minute {
        spec = spec.with_rate_limit_per_minute(rpm);
    }
    spec
}

/// `Send + Sync + 'static` mirror of the bus, intended to live on
/// `AppState`. Newtype rather than re-exporting the alias so future
/// behaviour (per-tenant routing, dropped-event metrics) lands here
/// without churning every consumer.
#[derive(Clone, Debug)]
pub struct TriggerLifecycleBus {
    inner: Arc<TriggerLifecycleEventBus>,
}

impl TriggerLifecycleBus {
    /// Construct a fresh bus with the given buffer capacity. Default
    /// callers should use 256 — the Phase 2 bus default.
    #[must_use]
    pub fn new(buffer: usize) -> Self {
        Self {
            inner: Arc::new(TriggerLifecycleEventBus::new(buffer)),
        }
    }

    /// Wrap an existing bus. Tests may share a bus across an actor
    /// pair (transport subscriber + producer mock).
    #[must_use]
    pub fn from_arc(inner: Arc<TriggerLifecycleEventBus>) -> Self {
        Self { inner }
    }

    /// Publish an event. Non-blocking; backpressure follows the bus
    /// policy (defaults to `DropOldest`).
    pub fn emit(&self, event: TriggerLifecycleEvent) {
        let _ = self.inner.emit(event);
    }

    /// Borrow the underlying bus — handed to
    /// [`TriggerLifecycleSubscriber::new`].
    #[must_use]
    pub fn bus(&self) -> Arc<TriggerLifecycleEventBus> {
        Arc::clone(&self.inner)
    }
}

impl Default for TriggerLifecycleBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    use nebula_action::webhook::providers;
    use nebula_engine::ActionRegistry;
    use nebula_storage::rows::WebhookActivationSpec;

    use crate::services::webhook::{WebhookTransport, WebhookTransportConfig};

    struct StaticSecrets {
        map: HashMap<String, Vec<u8>>,
    }

    #[async_trait]
    impl WebhookSecretResolver for StaticSecrets {
        async fn resolve(&self, secret_id: &str) -> Result<Vec<u8>, SecretResolutionError> {
            self.map.get(secret_id).cloned().ok_or_else(|| {
                Box::<dyn std::error::Error + Send + Sync>::from(format!(
                    "no secret for {secret_id}"
                ))
            })
        }
    }

    struct StubCtx;
    impl WebhookContextFactory for StubCtx {
        fn build(&self, _record: &WebhookActivationRecord) -> TriggerRuntimeContext {
            use tokio_util::sync::CancellationToken;
            TriggerRuntimeContext::new(
                Arc::new(
                    nebula_core::BaseContext::builder()
                        .cancellation(CancellationToken::new())
                        .build(),
                ),
                nebula_core::WorkflowId::new(),
                nebula_core::node_key!("lifecycle_test"),
            )
        }
    }

    fn record(slug: &str, kind: &str, secret_id: &str) -> WebhookActivationRecord {
        WebhookActivationRecord {
            trigger_id: vec![1u8; 16],
            coords: WebhookActivationCoords {
                org_slug: "acme".into(),
                workspace_slug: "ops".into(),
                trigger_slug: slug.into(),
            },
            spec: WebhookActivationSpec::new(kind, secret_id),
        }
    }

    fn registry_with_default_factories() -> Arc<ActionRegistry> {
        let registry = ActionRegistry::new();
        for factory in providers::default_factories() {
            registry.register_webhook_provider(factory);
        }
        Arc::new(registry)
    }

    fn make_subscriber() -> (
        TriggerLifecycleBus,
        WebhookTransport,
        TriggerLifecycleSubscriber,
    ) {
        let bus = TriggerLifecycleBus::default();
        let transport = WebhookTransport::new(WebhookTransportConfig::default());
        let registry = registry_with_default_factories();
        let mut secrets = HashMap::new();
        secrets.insert("cred_x".into(), b"super-secret".to_vec());
        let subscriber = TriggerLifecycleSubscriber::new(
            bus.bus(),
            transport.clone(),
            registry,
            Arc::new(StaticSecrets { map: secrets }),
            Arc::new(StubCtx),
        );
        (bus, transport, subscriber)
    }

    /// Sub-second wait that yields control between micro-checks. The
    /// subscriber is event-driven, so we just spin briefly until the
    /// transport's slug map reflects the change.
    async fn wait_until<F: Fn() -> bool>(condition: F) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        condition()
    }

    fn slug_count(transport: &WebhookTransport) -> usize {
        transport.slug_count()
    }

    #[tokio::test]
    async fn created_event_registers_activation() {
        let (bus, transport, subscriber) = make_subscriber();
        let _handle = subscriber.spawn();
        bus.emit(TriggerLifecycleEvent::Created(record(
            "stripe-prod",
            "generic",
            "cred_x",
        )));

        assert!(
            wait_until(|| slug_count(&transport) == 1).await,
            "expected slug map to grow to 1; got {}",
            slug_count(&transport)
        );
    }

    #[tokio::test]
    async fn deleted_event_drops_activation() {
        let (bus, transport, subscriber) = make_subscriber();
        let _handle = subscriber.spawn();
        bus.emit(TriggerLifecycleEvent::Created(record(
            "stripe-prod",
            "generic",
            "cred_x",
        )));
        assert!(wait_until(|| slug_count(&transport) == 1).await);

        bus.emit(TriggerLifecycleEvent::Deleted(WebhookActivationCoords {
            org_slug: "acme".into(),
            workspace_slug: "ops".into(),
            trigger_slug: "stripe-prod".into(),
        }));
        assert!(
            wait_until(|| slug_count(&transport) == 0).await,
            "expected slug map to drop to 0; got {}",
            slug_count(&transport)
        );
    }

    #[tokio::test]
    async fn updated_event_replaces_activation() {
        let (bus, transport, subscriber) = make_subscriber();
        let _handle = subscriber.spawn();
        bus.emit(TriggerLifecycleEvent::Created(record(
            "stripe-prod",
            "generic",
            "cred_x",
        )));
        assert!(wait_until(|| slug_count(&transport) == 1).await);

        bus.emit(TriggerLifecycleEvent::Updated(record(
            "stripe-prod",
            "generic",
            "cred_x",
        )));
        // Updated tears down + re-registers; the count should briefly
        // visit zero but settle at one.
        assert!(wait_until(|| slug_count(&transport) == 1).await);
    }

    #[tokio::test]
    async fn unknown_provider_kind_does_not_panic() {
        let (bus, transport, subscriber) = make_subscriber();
        let _handle = subscriber.spawn();
        bus.emit(TriggerLifecycleEvent::Created(record(
            "weird",
            "no-such-provider",
            "cred_x",
        )));
        // We can't observe a "rejected" state directly; just assert
        // the slug map stayed empty after a brief wait.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(slug_count(&transport), 0);
    }
}
