//! Storage-driven webhook bootstrap (webhook activation — ADR-0096, B-world).
//!
//! Reads active webhook activations from the spec-16 port store
//! (`WebhookActivationStore`), rebuilds each row's handler and routes it on
//! this process's [`WebhookTransport`]. The routing key is the stored token
//! hash, so an activation registered by another replica, or by this process
//! before a restart, answers its URL here too. Rows that are already routed
//! are left alone, which makes repeated passes cheap:
//! [`spawn_webhook_route_sync`] runs one at startup and then periodically.
//!
//! # Failure isolation
//!
//! [`bootstrap_webhook_activations`] is **best-effort**: a single
//! row that fails to resolve its secret, fails its factory build, or
//! would collide with an existing registration is logged at `warn` level
//! and counted in the returned [`BootstrapReport`]. The function only
//! returns `Err` for storage-layer failures (connection errors), so
//! the composition root can choose its own degraded-mode policy
//! (typically: log, surface `/healthz` degraded).
//!
//! The composition root invokes this function before handing
//! [`crate::AppState`] into [`crate::app::build_app`] — the `Router`
//! builder itself stays synchronous.

use std::{future::Future, sync::Arc, time::Duration};

use async_trait::async_trait;
use nebula_action::{
    BuiltWebhookHandler, FactoryError, TriggerRuntimeContext,
    webhook::factory::WebhookActivationSpec as ActionWebhookActivationSpec,
};
use nebula_engine::ActionRegistry;
use nebula_metrics::{
    MetricsRegistry, NEBULA_WEBHOOK_BOOTSTRAP_FAILURES_TOTAL, webhook_bootstrap_failure_reason,
};
use nebula_storage::webhook_activation::{
    WebhookActivationSpec as StorageWebhookActivationSpec, WebhookTimestampFormat,
};
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{
    Scope, StorageError as PortStorageError,
    dto::WebhookActivationRecord as PortWebhookActivationRecord,
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{key::WebhookKey, transport::WebhookTransport};
use crate::state::AppState;

/// Credential-neutral, secret-free failure returned by
/// [`WebhookSecretResolver::resolve`].
///
/// The enum deliberately carries no dynamic source or caller-provided text:
/// resolver implementations sit across a trust boundary, and their backend
/// errors may contain credential or provider material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SecretResolutionError {
    /// No secret exists in the caller's owner partition.
    #[error("webhook signing secret was not found")]
    NotFound,
    /// The backing credential system could not resolve material now.
    #[error("webhook signing secret is unavailable")]
    Unavailable,
    /// Stored material does not satisfy the signing-secret format contract.
    #[error("webhook signing secret material is invalid")]
    InvalidMaterial,
}

/// Resolves a credential identifier (storage-layer string) to the
/// raw HMAC secret bytes consumed by the factory.
///
/// Production deployments provide an owner-scoped adapter from their
/// composition root; tests may wire an in-memory map. Returning an empty
/// `Vec` is **not** allowed — the factory would then build a fail-closed
/// handler. Surface the misconfiguration as
/// [`SecretResolutionError::InvalidMaterial`].
///
/// # Tenant isolation
///
/// `scope` is mandatory. Implementations must use it as the authoritative
/// owner partition and must not offer an unscoped lookup fallback.
#[async_trait]
pub trait WebhookSecretResolver: Send + Sync {
    /// Resolve `secret_id` to raw secret bytes, scoped to `scope`.
    ///
    /// # Errors
    ///
    /// Returns a closed, secret-free error on lookup failure. The bootstrap
    /// wraps it in [`BootstrapError::SecretResolution`].
    async fn resolve(
        &self,
        scope: &Scope,
        secret_id: &str,
    ) -> Result<Vec<u8>, SecretResolutionError>;
}

/// Result of [`bootstrap_webhook_activations`].
///
/// `loaded` is the count of active rows routed on the transport after the
/// pass, `restored` the subset this pass newly routed. `skipped` is the
/// count of rows that surfaced a non-storage error (unroutable row, decode
/// mismatch, factory rejection, secret resolution failure).
#[derive(Debug, Clone, Copy, Default)]
pub struct BootstrapReport {
    /// Active rows routed on the transport after this pass.
    pub loaded: usize,
    /// Rows this pass newly routed (the rest were already routed).
    pub restored: usize,
    /// Rows that returned a non-storage failure and were skipped.
    pub skipped: usize,
}

/// Failure modes for the bootstrap pathway (B-world port store).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BootstrapError {
    /// `WebhookActivationStore::list_all_active` returned a storage error.
    #[error("port store list_all_active failed: {0}")]
    Storage(#[from] PortStorageError),
    /// The trigger-config lookup returned no `WebhookActivationSpec` for the
    /// row's `trigger_id`. The activation row is skipped.
    #[error("no webhook_activation spec found in trigger config for trigger_id={trigger_id}")]
    MissingSpec {
        /// Trigger id that had no spec in `triggers.config`.
        trigger_id: String,
    },
    /// The `TriggerSpecLookup` call itself failed (storage / decode error).
    /// Distinct from [`Self::MissingSpec`] (spec absent) and
    /// [`Self::SecretResolution`] (spec found, credential lookup failed).
    #[error("trigger spec lookup failed for trigger_id={trigger_id}: {source}")]
    SpecLookup {
        /// Trigger id whose spec lookup failed.
        trigger_id: String,
        /// Underlying lookup error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Could not resolve the credential reference to raw secret bytes.
    #[error("failed to resolve secret '{secret_id}': {source}")]
    SecretResolution {
        /// Storage-layer credential identifier that failed to resolve.
        secret_id: String,
        /// Underlying cause.
        #[source]
        source: SecretResolutionError,
    },
    /// The row carries no routable capability token: its `slug` is not the
    /// activation UUID or its `token_hash` is unset. Rows written by
    /// `activate_and_persist` always carry both.
    #[error("activation row for trigger_id={trigger_id} has no routable capability token")]
    Unroutable {
        /// Trigger id of the unroutable row.
        trigger_id: String,
    },
    /// The row's identity could not be turned into a trigger context.
    #[error("cannot build a trigger context for trigger_id={trigger_id}: {source}")]
    Context {
        /// Trigger id of the row.
        trigger_id: String,
        /// Which part of the identity is invalid.
        #[source]
        source: WebhookContextError,
    },
    /// No factory registered for the spec's `action_kind`.
    #[error("no factory registered for kind '{0}'")]
    UnknownProvider(String),
    /// Factory rejected the spec.
    #[error("factory build failed for kind '{kind}': {source}")]
    Factory {
        /// Provider kind.
        kind: String,
        /// Underlying [`FactoryError`].
        #[source]
        source: FactoryError,
    },
}

/// Constructs a per-activation [`TriggerRuntimeContext`] from a B-world
/// activation record.
///
/// The B-world record carries `scope`, `trigger_id`, and `workflow_id`; the
/// factory converts these into a [`TriggerRuntimeContext`] template the
/// transport clones on every dispatch.
pub trait WebhookActivationContextFactory: Send + Sync {
    /// Build a fresh ctx template for the B-world activation.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookContextError`] when the record's workflow or trigger
    /// identity is missing or malformed.
    fn build(
        &self,
        record: &PortWebhookActivationRecord,
    ) -> Result<TriggerRuntimeContext, WebhookContextError>;
}

/// A registry holding the first-party webhook provider factories
/// (`nebula_action::webhook::providers::default_factories`) and nothing else,
/// for [`AppState::with_webhook_providers`].
#[must_use]
pub fn default_webhook_providers() -> Arc<ActionRegistry> {
    let registry = ActionRegistry::new();
    for factory in nebula_action::webhook::providers::default_factories() {
        registry.register_webhook_provider(factory);
    }
    Arc::new(registry)
}

/// Why an activation record cannot become a trigger context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WebhookContextError {
    /// The record names no workflow.
    #[error("activation record names no workflow")]
    MissingWorkflow,
    /// The record's workflow id is not a workflow id.
    #[error("activation record workflow id is malformed")]
    InvalidWorkflowId,
    /// The record's trigger id is not a node key.
    #[error("activation record trigger id is not a node key")]
    InvalidTriggerId,
    /// The base context could not be built.
    #[error("trigger base context could not be built")]
    BaseContext,
}

/// Builds the trigger context of an activation from its record alone: the
/// tenant, workflow and trigger node the record names, a system principal,
/// and default capabilities.
///
/// Durable (Prod) dispatch starts executions from the row through the start
/// service, not through the context's emitter, so nothing deployment-specific
/// is needed here.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordWebhookContextFactory;

impl WebhookActivationContextFactory for RecordWebhookContextFactory {
    fn build(
        &self,
        record: &PortWebhookActivationRecord,
    ) -> Result<TriggerRuntimeContext, WebhookContextError> {
        let workflow_id = record
            .workflow_id
            .as_deref()
            .ok_or(WebhookContextError::MissingWorkflow)?;
        let workflow_id = nebula_core::WorkflowId::parse(workflow_id)
            .map_err(|_| WebhookContextError::InvalidWorkflowId)?;
        let trigger = nebula_core::NodeKey::new(&record.trigger_id)
            .map_err(|_| WebhookContextError::InvalidTriggerId)?;
        let scope = nebula_core::scope::Scope {
            org_id: nebula_core::OrgId::parse(&record.scope.org_id).ok(),
            workspace_id: nebula_core::WorkspaceId::parse(&record.scope.workspace_id).ok(),
            ..Default::default()
        };
        let base = nebula_core::BaseContext::builder(scope)
            .principal(nebula_core::scope::Principal::System)
            .cancellation(CancellationToken::new())
            .build()
            .map_err(|_| WebhookContextError::BaseContext)?;
        Ok(TriggerRuntimeContext::new(
            Arc::new(base),
            workflow_id,
            trigger,
        ))
    }
}

/// Return type for [`TriggerSpecLookup::lookup`] — a boxed future that resolves
/// to an optional spec or a storage error.
///
/// Extracted as a type alias to keep the trait signature readable and to
/// satisfy `clippy::type_complexity`.
type SpecLookupFuture<'async_trait> = std::pin::Pin<
    Box<
        dyn Future<
                Output = Result<
                    Option<StorageWebhookActivationSpec>,
                    Box<dyn std::error::Error + Send + Sync>,
                >,
            > + Send
            + 'async_trait,
    >,
>;

/// Looks up the `WebhookActivationSpec` stored in `triggers.config` for a
/// given trigger id, **owner-scoped**.
///
/// B's lean row carries only routing/token/scope/workflow/mode; the handler-build
/// inputs (`provider`, `secret_id`, replay knobs) live in
/// `triggers.config.webhook_activation`. This trait abstracts the lookup so the
/// bootstrap is independent of the DB query shape.
///
/// # Scope enforcement
///
/// `scope` is mandatory: the implementation must partition the lookup by the
/// activation row's `scope` so a `trigger_id` from one tenant never resolves
/// a spec belonging to another (BOLA/IDOR closed by construction, mirroring
/// `ScopedTriggerStore`).
pub trait TriggerSpecLookup: Send + Sync {
    /// Resolve the webhook activation spec for `trigger_id` within `scope`.
    ///
    /// Returns `None` when the trigger has no `webhook_activation` namespace in
    /// its config, or the trigger does not exist in `scope`.
    ///
    /// # Errors
    ///
    /// Returns a boxed error on storage failure.
    fn lookup<'life0, 'life1, 'life2, 'async_trait>(
        &'life0 self,
        scope: &'life1 Scope,
        trigger_id: &'life2 str,
    ) -> SpecLookupFuture<'async_trait>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait;
}

/// Walk the B-world port store's active webhook activations and route each on
/// `transport`. Returns a [`BootstrapReport`] with loaded/restored/skipped
/// counts.
///
/// A row already routed on `transport` is counted as loaded without being
/// rebuilt. Any other row is rebuilt from its trigger spec (spec lookup,
/// secret resolution, factory build) and routed under its stored token hash.
///
/// Failure isolation is per-row: per-row failures are logged and counted;
/// storage failures bubble out.
///
/// # Errors
///
/// Returns [`BootstrapError::Storage`] only when the underlying port store
/// call fails. Per-row failures are absorbed into `report.skipped`.
pub async fn bootstrap_webhook_activations(
    transport: &WebhookTransport,
    store: &dyn WebhookActivationStore,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
    metrics: Option<&MetricsRegistry>,
) -> Result<BootstrapReport, BootstrapError> {
    tracing::debug!(
        target: "nebula::api::webhook::bootstrap",
        "loading active webhook activations from port store"
    );

    let records = store.list_all_active().await.inspect_err(|_| {
        record_bootstrap_failure(metrics, webhook_bootstrap_failure_reason::STORAGE);
    })?;
    let total = records.len();
    let mut report = BootstrapReport::default();

    for record in &records {
        match route_one(
            transport,
            record,
            registry,
            secrets,
            ctx_factory,
            spec_lookup,
        )
        .await
        {
            Ok(restored) => {
                report.loaded += 1;
                if restored {
                    report.restored += 1;
                }
            },
            Err(err) => {
                report.skipped += 1;
                record_bootstrap_failure(metrics, bootstrap_failure_reason(&err));
                tracing::warn!(
                    target: "nebula::api::webhook::bootstrap",
                    failure_reason = bootstrap_failure_reason(&err),
                    trigger_id = %record.trigger_id,
                    "skipping webhook activation"
                );
            },
        }
    }

    debug_assert!(
        report.loaded + report.skipped == total,
        "bootstrap accounting must equal list_all_active count"
    );

    tracing::debug!(
        target: "nebula::api::webhook::bootstrap",
        loaded = report.loaded,
        restored = report.restored,
        skipped = report.skipped,
        total,
        source = "port_store",
        "webhook routes synchronized"
    );
    Ok(report)
}

/// The routing key a persisted row answers to: its activation UUID (stored as
/// the slug) and its token hash.
fn routing_key(record: &PortWebhookActivationRecord) -> Result<WebhookKey, BootstrapError> {
    let unroutable = || BootstrapError::Unroutable {
        trigger_id: record.trigger_id.clone(),
    };
    if record.token_hash == [0u8; 32] {
        return Err(unroutable());
    }
    let uuid = Uuid::parse_str(&record.slug).map_err(|_| unroutable())?;
    Ok(WebhookKey::from_token_hash(uuid, record.token_hash))
}

/// Route one B-world activation row on `transport`: spec lookup + secret
/// resolution + factory build, then insert under the stored token hash.
/// Returns whether this call routed it (`false` when it was already routed).
async fn route_one(
    transport: &WebhookTransport,
    record: &PortWebhookActivationRecord,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
) -> Result<bool, BootstrapError> {
    let key = routing_key(record)?;
    if transport.is_routed(&key) {
        return Ok(false);
    }

    // Use spec_trigger_id (the `triggers` PK, `trg_` prefix) as the spec
    // lookup key — NOT trigger_id (the dispatch routing NodeKey).  Legacy rows
    // written before ADR-0101 have spec_trigger_id = None; skip them with
    // MissingSpec so we never confuse a NodeKey for a spec-row PK.
    let spec_pk = record.spec_trigger_id.as_deref().ok_or_else(|| {
        tracing::warn!(
            target: "nebula::api::webhook::bootstrap",
            trigger_id = %record.trigger_id,
            "activation row has no spec_trigger_id (legacy row written before ADR-0101); \
             skipping bootstrap reconstruct for this activation"
        );
        BootstrapError::MissingSpec {
            trigger_id: record.trigger_id.clone(),
        }
    })?;

    let spec = spec_lookup
        .lookup(&record.scope, spec_pk)
        .await
        .map_err(|source| BootstrapError::SpecLookup {
            trigger_id: record.trigger_id.clone(),
            source,
        })?
        .ok_or_else(|| BootstrapError::MissingSpec {
            trigger_id: record.trigger_id.clone(),
        })?;

    let secret = secrets
        .resolve(&record.scope, &spec.secret_id)
        .await
        .and_then(super::signing_secret::validate_resolved_secret)
        .map_err(|source| BootstrapError::SecretResolution {
            secret_id: spec.secret_id.clone(),
            source,
        })?;

    let action_spec = into_action_spec(&spec, secret);

    let factory = registry
        .lookup_webhook_factory(&spec.provider)
        .ok_or_else(|| BootstrapError::UnknownProvider(spec.provider.clone()))?;

    let BuiltWebhookHandler { handler, config } =
        factory
            .build(&action_spec)
            .map_err(|source| BootstrapError::Factory {
                kind: spec.provider.clone(),
                source,
            })?;
    let ctx = ctx_factory
        .build(record)
        .map_err(|source| BootstrapError::Context {
            trigger_id: record.trigger_id.clone(),
            source,
        })?;

    // A concurrent pass may have routed the same row meanwhile; either entry
    // serves it, so losing that race is not an error.
    let restored = transport.restore(key, handler, config, ctx);
    tracing::debug!(
        target: "nebula::api::webhook::bootstrap",
        trigger_id = %record.trigger_id,
        scope = ?record.scope,
        restored,
        "webhook activation routed from the port store"
    );
    Ok(restored)
}

/// Route every active persisted activation on the transport attached to
/// `state`.
///
/// Returns `Ok(None)` when `state` lacks a webhook transport or any component
/// a rebuild needs (activation store, provider registry, secret resolver,
/// context factory, spec lookup): such a process does not serve webhooks.
///
/// # Errors
///
/// Returns [`BootstrapError::Storage`] when listing active rows fails.
pub async fn sync_webhook_routes(
    state: &AppState,
) -> Result<Option<BootstrapReport>, BootstrapError> {
    let (
        Some(transport),
        Some(store),
        Some(registry),
        Some(secrets),
        Some(ctx_factory),
        Some(spec_lookup),
    ) = (
        state.webhook_transport.as_ref(),
        state.webhook_activation_store.as_ref(),
        state.webhook_provider_registry(),
        state.webhook_secret_resolver.as_ref(),
        state.webhook_ctx_factory_b.as_ref(),
        state.webhook_spec_lookup.as_ref(),
    )
    else {
        return Ok(None);
    };
    bootstrap_webhook_activations(
        transport,
        store.as_ref(),
        registry,
        secrets.as_ref(),
        ctx_factory.as_ref(),
        spec_lookup.as_ref(),
        state.metrics_registry.as_deref(),
    )
    .await
    .map(Some)
}

/// Keep this process's webhook routes in step with the activation store:
/// one pass now, then one every `every`, until `shutdown` is cancelled.
///
/// Activations registered on another replica, or on this process before a
/// restart, become routable here within one interval. A failed pass is
/// logged and retried at the next tick. Returns `None` without spawning when
/// `state` cannot route webhooks (see [`sync_webhook_routes`]).
pub fn spawn_webhook_route_sync(
    state: AppState,
    every: Duration,
    shutdown: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    if state.webhook_transport.is_none()
        || state.webhook_activation_store.is_none()
        || state.webhook_provider_registry().is_none()
        || state.webhook_secret_resolver.is_none()
        || state.webhook_ctx_factory_b.is_none()
        || state.webhook_spec_lookup.is_none()
    {
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = ticks.tick() => {},
            }
            match sync_webhook_routes(&state).await {
                Ok(Some(report)) if report.restored > 0 || report.skipped > 0 => {
                    tracing::info!(
                        target: "nebula::api::webhook::bootstrap",
                        loaded = report.loaded,
                        restored = report.restored,
                        skipped = report.skipped,
                        "webhook routes synchronized from the activation store"
                    );
                },
                Ok(_) => {},
                Err(error) => {
                    tracing::warn!(
                        target: "nebula::api::webhook::bootstrap",
                        error = %error,
                        "webhook route sync failed; retrying at the next interval"
                    );
                },
            }
        }
    }))
}

fn into_action_spec(
    storage: &StorageWebhookActivationSpec,
    secret: Vec<u8>,
) -> ActionWebhookActivationSpec {
    let mut spec = ActionWebhookActivationSpec::new(storage.provider.clone(), secret);
    if let Some(secs) = storage.replay_window_secs {
        spec = spec.with_replay_window_secs(secs);
    }
    if let Some(header) = storage.timestamp_header.as_ref() {
        spec = spec.with_timestamp_header(header.clone());
    }
    if let Some(format) = storage.timestamp_format {
        spec = spec.with_timestamp_format(map_timestamp_format(format));
    }
    if let Some(config) = storage.provider_config.clone() {
        spec = spec.with_provider_config(config);
    }
    if let Some(rpm) = storage.rate_limit_per_minute {
        spec = spec.with_rate_limit_per_minute(rpm);
    }
    spec
}

/// Translate the storage-layer timestamp encoding enum to the
/// action-layer one. Both crates ship `#[non_exhaustive]` enums; this
/// helper is the single conversion seam between them.
fn map_timestamp_format(format: WebhookTimestampFormat) -> nebula_action::webhook::TimestampFormat {
    match format {
        WebhookTimestampFormat::UnixSeconds => nebula_action::webhook::TimestampFormat::UnixSeconds,
        WebhookTimestampFormat::UnixMillis => nebula_action::webhook::TimestampFormat::UnixMillis,
        WebhookTimestampFormat::Rfc3339 => nebula_action::webhook::TimestampFormat::Rfc3339,
        // Both enums are #[non_exhaustive] — fall back to the default
        // (Unix seconds) if storage adds a variant the action layer
        // does not yet know about.
        _ => nebula_action::webhook::TimestampFormat::UnixSeconds,
    }
}

fn bootstrap_failure_reason(err: &BootstrapError) -> &'static str {
    match err {
        BootstrapError::Storage(_) => webhook_bootstrap_failure_reason::STORAGE,
        BootstrapError::Unroutable { .. } | BootstrapError::Context { .. } => {
            webhook_bootstrap_failure_reason::DECODE
        },
        BootstrapError::MissingSpec { .. }
        | BootstrapError::SpecLookup { .. }
        | BootstrapError::SecretResolution { .. }
        | BootstrapError::UnknownProvider(_)
        | BootstrapError::Factory { .. } => webhook_bootstrap_failure_reason::FACTORY,
    }
}

fn record_bootstrap_failure(metrics: Option<&MetricsRegistry>, reason: &'static str) {
    let Some(reg) = metrics else { return };
    let labels = reg.interner().single("reason", reason);
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_BOOTSTRAP_FAILURES_TOTAL, &labels) {
        c.inc();
    }
}

// Keep a minimal unused-import-free test that proves the trait bounds hold.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn marker_traits_are_satisfied() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BootstrapReport>();
        let _ = Mutex::new(BootstrapReport::default());
    }
}
