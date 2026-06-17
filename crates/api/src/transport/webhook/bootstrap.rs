//! Storage-driven webhook bootstrap (webhook activation).
//!
//! Walks every active operator-configured webhook activation in
//! storage, instantiates the matching `WebhookActionFactory`, and
//! registers the resulting handler in the [`WebhookTransport`] under
//! its slug coordinates.
//!
//! # Failure isolation
//!
//! [`bootstrap_webhook_activations`] is **best-effort**: a single
//! row that fails to resolve its secret, fails its factory build, or
//! collides with an existing registration is logged at `warn` level
//! and counted in the returned [`BootstrapReport`]. The function only
//! returns `Err` for storage-layer failures (connection errors), so
//! the composition root can choose its own degraded-mode policy
//! (typically: log, leave the slug map empty, surface `/healthz`
//! degraded). See webhook activation for the rationale.
//!
//! The composition root invokes this function before handing
//! [`crate::AppState`] into [`crate::app::build_app`] — the `Router`
//! builder itself stays synchronous.

use std::{future::Future, sync::Arc};

use async_trait::async_trait;
use nebula_action::{
    BuiltWebhookHandler, FactoryError, TriggerHandler, TriggerRuntimeContext,
    webhook::factory::WebhookActivationSpec as ActionWebhookActivationSpec,
};
use nebula_engine::ActionRegistry;
use nebula_metrics::{
    MetricsRegistry, NEBULA_WEBHOOK_BOOTSTRAP_FAILURES_TOTAL, webhook_bootstrap_failure_reason,
};
use nebula_storage::{
    StorageError,
    repos::WebhookActivationRepo,
    rows::{
        WebhookActivationRecord, WebhookActivationSpec as StorageWebhookActivationSpec,
        WebhookTimestampFormat,
    },
};
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{
    StorageError as PortStorageError, dto::WebhookActivationRecord as PortWebhookActivationRecord,
};
use thiserror::Error;

use super::{
    key::TriggerCoordinates,
    transport::{ActivationError, WebhookTransport},
};

/// Boxed error type returned by [`WebhookSecretResolver::resolve`].
/// The bootstrap wraps it in [`BootstrapError::SecretResolution`]
/// alongside the failing `secret_id`.
pub type SecretResolutionError = Box<dyn std::error::Error + Send + Sync>;

/// Resolves a credential identifier (storage-layer string) to the
/// raw HMAC secret bytes consumed by the factory.
///
/// Production deployments wire this through `nebula-credential`'s
/// snapshot path; tests wire an in-memory map. Returning an empty
/// `Vec` is **not** allowed — the factory would then build a
/// fail-closed handler. Surface the misconfiguration as a typed
/// error inside the boxed return.
#[async_trait]
pub trait WebhookSecretResolver: Send + Sync {
    /// Resolve `secret_id` to raw secret bytes.
    ///
    /// # Errors
    ///
    /// Returns a boxed error on lookup failure. The bootstrap wraps
    /// it in [`BootstrapError::SecretResolution`].
    async fn resolve(&self, secret_id: &str) -> Result<Vec<u8>, SecretResolutionError>;
}

/// Constructs a per-activation [`TriggerRuntimeContext`] template.
///
/// Storage rows do not carry the `BaseContext`/`WorkflowId`/node-key
/// triple needed to build a [`TriggerRuntimeContext`]; the
/// composition root supplies that mapping by implementing this
/// trait. The transport clones the returned ctx on every dispatch.
pub trait WebhookContextFactory: Send + Sync {
    /// Build a fresh ctx template for the activation.
    fn build(&self, record: &WebhookActivationRecord) -> TriggerRuntimeContext;
}

/// Result of [`bootstrap_webhook_activations`].
///
/// `loaded` is the count of slug activations that successfully made
/// it into the transport. `skipped` is the count of rows that
/// surfaced a non-storage error (decode mismatch, factory rejection,
/// secret resolution failure, duplicate registration).
#[derive(Debug, Clone, Copy, Default)]
pub struct BootstrapReport {
    /// Slug activations registered in the transport.
    pub loaded: usize,
    /// Rows that returned a non-storage failure and were skipped.
    pub skipped: usize,
}

/// Materialised activation ready for `WebhookTransport::replace_slug_map`.
///
/// Used by the admin reload endpoint (E3) so the slug map swap is
/// atomic — caller collects the full set, then hands it back to the
/// transport in one call.
pub type ResolvedActivation = (
    TriggerCoordinates,
    Arc<dyn TriggerHandler>,
    nebula_action::WebhookConfig,
    TriggerRuntimeContext,
);

/// Failure modes for the bootstrap pathway.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BootstrapError {
    /// `WebhookActivationRepo::list_active` returned a transport-level
    /// error (DB connection, query failure). The transport is left
    /// untouched; admin reload (E3) can retry.
    #[error("storage list_active failed: {0}")]
    Storage(#[from] StorageError),
    /// Could not resolve the credential reference to raw secret
    /// bytes. The activation row is skipped.
    #[error("failed to resolve secret '{secret_id}': {source}")]
    SecretResolution {
        /// Storage-layer credential identifier that failed to resolve.
        secret_id: String,
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// No factory registered for the spec's `action_kind`. The
    /// activation row is skipped.
    #[error("no factory registered for kind '{0}'")]
    UnknownProvider(String),
    /// Factory rejected the spec — provider-specific fields missing
    /// or malformed.
    #[error("factory build failed for kind '{kind}': {source}")]
    Factory {
        /// Provider kind reported by the factory.
        kind: String,
        /// Underlying [`FactoryError`].
        #[source]
        source: FactoryError,
    },
    /// Two storage rows resolved to the same slug coordinates. The
    /// second row is skipped — DB-level uniqueness on `webhook_path`
    /// prevents this in production but the bootstrap defends in
    /// depth.
    #[error("duplicate slug registration for {coords:?}")]
    DuplicateRegistration {
        /// Slug coordinates that were already registered.
        coords: TriggerCoordinates,
    },
}

/// Walk the storage layer's active webhook activations and register
/// each in the transport.
///
/// Returns a [`BootstrapReport`] with per-row outcomes. Storage
/// failures bubble out as [`BootstrapError::Storage`]; per-row
/// failures (factory rejection, secret resolution miss, duplicate)
/// are logged and counted in `report.skipped`.
///
/// `metrics` (when `Some`) bumps
/// [`nebula_metrics::NEBULA_WEBHOOK_BOOTSTRAP_FAILURES_TOTAL`] per
/// skipped row, labeled by reason.
///
/// # Errors
///
/// Returns [`BootstrapError::Storage`] only when the underlying
/// storage call fails. Per-row failures are absorbed.
pub async fn bootstrap_webhook_activations(
    repo: &dyn WebhookActivationRepo,
    registry: &ActionRegistry,
    transport: &WebhookTransport,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookContextFactory,
    metrics: Option<&MetricsRegistry>,
) -> Result<BootstrapReport, BootstrapError> {
    tracing::debug!(target: "nebula::api::webhook::bootstrap", "loading active webhook activations");

    let records = repo.list_active().await.inspect_err(|_| {
        record_bootstrap_failure(metrics, webhook_bootstrap_failure_reason::STORAGE);
    })?;
    let total = records.len();
    let mut report = BootstrapReport::default();

    for record in records {
        match register_one(record, registry, transport, secrets, ctx_factory).await {
            Ok(()) => report.loaded += 1,
            Err(err) => {
                report.skipped += 1;
                record_bootstrap_failure(metrics, bootstrap_failure_reason(&err));
                tracing::warn!(
                    target: "nebula::api::webhook::bootstrap",
                    error = %err,
                    "skipping webhook activation"
                );
            },
        }
    }

    debug_assert!(
        report.loaded + report.skipped == total,
        "bootstrap accounting must equal storage list_active count"
    );

    tracing::info!(
        target: "nebula::api::webhook::bootstrap",
        loaded = report.loaded,
        skipped = report.skipped,
        total,
        source = "storage",
        "webhook bootstrap complete"
    );
    Ok(report)
}

/// Build the full set of resolved activations from storage without
/// touching the transport. The admin reload endpoint (E3) uses this
/// to materialise an atomic swap via
/// `WebhookTransport::replace_slug_map`.
///
/// Per-row failures are absorbed into the returned report's
/// `skipped` field; storage failures bubble out via
/// [`BootstrapError::Storage`].
///
/// # Errors
///
/// Returns [`BootstrapError::Storage`] only when the underlying
/// storage call fails.
pub async fn collect_webhook_activations(
    repo: &dyn WebhookActivationRepo,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookContextFactory,
) -> Result<(Vec<ResolvedActivation>, BootstrapReport), BootstrapError> {
    let records = repo.list_active().await?;
    let mut report = BootstrapReport::default();
    let mut activations: Vec<ResolvedActivation> = Vec::with_capacity(records.len());
    for record in records {
        match resolve_one(&record, registry, secrets, ctx_factory).await {
            Ok(resolved) => {
                report.loaded += 1;
                activations.push(resolved);
            },
            Err(err) => {
                report.skipped += 1;
                tracing::warn!(
                    target: "nebula::api::webhook::bootstrap",
                    error = %err,
                    "skipping webhook activation during collect"
                );
            },
        }
    }
    Ok((activations, report))
}

async fn resolve_one(
    record: &WebhookActivationRecord,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookContextFactory,
) -> Result<ResolvedActivation, BootstrapError> {
    let factory = registry
        .lookup_webhook_factory(&record.spec.action_kind)
        .ok_or_else(|| BootstrapError::UnknownProvider(record.spec.action_kind.clone()))?;

    let secret = secrets
        .resolve(&record.spec.secret_id)
        .await
        .map_err(|source| BootstrapError::SecretResolution {
            secret_id: record.spec.secret_id.clone(),
            source,
        })?;
    let action_spec = into_action_spec(&record.spec, secret);

    let BuiltWebhookHandler { handler, config } =
        factory
            .build(&action_spec)
            .map_err(|source| BootstrapError::Factory {
                kind: record.spec.action_kind.clone(),
                source,
            })?;

    let coords = TriggerCoordinates::new(
        &record.coords.org_slug,
        &record.coords.workspace_slug,
        &record.coords.trigger_slug,
    );
    let ctx = ctx_factory.build(record);
    Ok((coords, handler, config, ctx))
}

async fn register_one(
    record: WebhookActivationRecord,
    registry: &ActionRegistry,
    transport: &WebhookTransport,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookContextFactory,
) -> Result<(), BootstrapError> {
    let factory = registry
        .lookup_webhook_factory(&record.spec.action_kind)
        .ok_or_else(|| BootstrapError::UnknownProvider(record.spec.action_kind.clone()))?;

    let secret = secrets
        .resolve(&record.spec.secret_id)
        .await
        .map_err(|source| BootstrapError::SecretResolution {
            secret_id: record.spec.secret_id.clone(),
            source,
        })?;
    let action_spec = into_action_spec(&record.spec, secret);

    let BuiltWebhookHandler { handler, config } =
        factory
            .build(&action_spec)
            .map_err(|source| BootstrapError::Factory {
                kind: record.spec.action_kind.clone(),
                source,
            })?;

    let coords = TriggerCoordinates::new(
        &record.coords.org_slug,
        &record.coords.workspace_slug,
        &record.coords.trigger_slug,
    );
    let ctx = ctx_factory.build(&record);

    let action_kind = record.spec.action_kind.clone();
    register_with_transport(transport, coords.clone(), handler, config, ctx).map_err(|err| {
        match err {
            ActivationError::DuplicateRegistration => {
                BootstrapError::DuplicateRegistration { coords }
            },
            other => BootstrapError::Factory {
                kind: action_kind,
                source: FactoryError::InvalidSpec {
                    kind: "transport",
                    reason: other.to_string(),
                },
            },
        }
    })?;

    tracing::debug!(
        target: "nebula::api::webhook::bootstrap",
        org = %record.coords.org_slug,
        workspace = %record.coords.workspace_slug,
        trigger_slug = %record.coords.trigger_slug,
        action_kind = %record.spec.action_kind,
        "webhook activation registered"
    );
    Ok(())
}

fn register_with_transport(
    transport: &WebhookTransport,
    coords: TriggerCoordinates,
    handler: Arc<dyn TriggerHandler>,
    config: nebula_action::WebhookConfig,
    ctx: TriggerRuntimeContext,
) -> Result<(), ActivationError> {
    transport.activate_slug(coords, handler, config, ctx)
}

/// `pub(super)` re-export for the lifecycle subscriber so storage →
/// action spec mapping has a single seam.
pub(super) fn storage_spec_into_action_spec(
    storage: &StorageWebhookActivationSpec,
    secret: Vec<u8>,
) -> ActionWebhookActivationSpec {
    into_action_spec(storage, secret)
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
        BootstrapError::SecretResolution { .. } => webhook_bootstrap_failure_reason::FACTORY,
        BootstrapError::UnknownProvider(_) | BootstrapError::Factory { .. } => {
            webhook_bootstrap_failure_reason::FACTORY
        },
        BootstrapError::DuplicateRegistration { .. } => webhook_bootstrap_failure_reason::FACTORY,
    }
}

fn record_bootstrap_failure(metrics: Option<&MetricsRegistry>, reason: &'static str) {
    let Some(reg) = metrics else { return };
    let labels = reg.interner().single("reason", reason);
    if let Ok(c) = reg.counter_labeled(NEBULA_WEBHOOK_BOOTSTRAP_FAILURES_TOTAL, &labels) {
        c.inc();
    }
}

// ── B-world bootstrap (ADR-0096 — port store + spec-16 aligned) ─────────────

/// Failure modes for the B-world bootstrap pathway.
///
/// Mirrors [`BootstrapError`] but sources rows from the spec-16 port store
/// (`WebhookActivationStore`) rather than the A-world `WebhookActivationRepo`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BootstrapErrorB {
    /// `WebhookActivationStore::list_all_active` returned a storage error.
    #[error("port store list_all_active failed: {0}")]
    Storage(#[from] PortStorageError),
    /// The trigger-config lookup returned no `WebhookActivationSpec` for the
    /// row's `trigger_id`.  The activation row is skipped.
    #[error("no webhook_activation spec found in trigger config for trigger_id={trigger_id}")]
    MissingSpec {
        /// Trigger id that had no spec in `triggers.config`.
        trigger_id: String,
    },
    /// Could not resolve the credential reference to raw secret bytes.
    #[error("failed to resolve secret '{secret_id}': {source}")]
    SecretResolution {
        /// Storage-layer credential identifier that failed to resolve.
        secret_id: String,
        /// Underlying cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
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
    /// Two active rows resolved to the same slug coordinates.
    #[error("duplicate slug registration for {coords:?}")]
    DuplicateRegistration {
        /// Slug coordinates that were already registered.
        coords: TriggerCoordinates,
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
    fn build(&self, record: &PortWebhookActivationRecord) -> TriggerRuntimeContext;
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
/// given trigger id.
///
/// B's lean row carries only routing/token/scope/workflow/mode; the handler-build
/// inputs (`action_kind`, `secret_id`, replay knobs) live in
/// `triggers.config.webhook_activation`.  This trait abstracts the lookup so the
/// bootstrap is independent of the DB query shape.
pub trait TriggerSpecLookup: Send + Sync {
    /// Resolve the webhook activation spec for `trigger_id`.
    ///
    /// Returns `None` when the trigger has no `webhook_activation` namespace in
    /// its config, or the trigger does not exist.
    ///
    /// # Errors
    ///
    /// Returns a boxed error on storage failure.
    fn lookup<'life0, 'life1, 'async_trait>(
        &'life0 self,
        trigger_id: &'life1 str,
    ) -> SpecLookupFuture<'async_trait>
    where
        'life0: 'async_trait,
        'life1: 'async_trait;
}

/// Walk the B-world port store's active webhook activations and register each
/// in the transport.
///
/// Reads routing rows from `WebhookActivationStore::list_all_active`, then
/// fetches the handler-build spec from `spec_lookup` (which reads
/// `triggers.config.webhook_activation` keyed by each row's `trigger_id`).
///
/// Failure isolation mirrors [`bootstrap_webhook_activations`]: per-row
/// failures are logged and counted; storage failures bubble out.
///
/// # Errors
///
/// Returns [`BootstrapErrorB::Storage`] only when the underlying port store
/// call fails. Per-row failures are absorbed into `report.skipped`.
pub async fn bootstrap_webhook_activations_b(
    store: &dyn WebhookActivationStore,
    registry: &ActionRegistry,
    transport: &WebhookTransport,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
    metrics: Option<&MetricsRegistry>,
) -> Result<BootstrapReport, BootstrapErrorB> {
    tracing::debug!(
        target: "nebula::api::webhook::bootstrap_b",
        "loading active webhook activations from port store"
    );

    let records = store.list_all_active().await.inspect_err(|_| {
        record_bootstrap_failure(metrics, webhook_bootstrap_failure_reason::STORAGE);
    })?;
    let total = records.len();
    let mut report = BootstrapReport::default();

    for record in records {
        match register_one_b(
            &record,
            registry,
            transport,
            secrets,
            ctx_factory,
            spec_lookup,
        )
        .await
        {
            Ok(()) => report.loaded += 1,
            Err(err) => {
                report.skipped += 1;
                record_bootstrap_failure(metrics, bootstrap_failure_reason_b(&err));
                tracing::warn!(
                    target: "nebula::api::webhook::bootstrap_b",
                    error = %err,
                    trigger_id = %record.trigger_id,
                    "skipping B-world webhook activation"
                );
            },
        }
    }

    debug_assert!(
        report.loaded + report.skipped == total,
        "B-world bootstrap accounting must equal list_all_active count"
    );

    tracing::info!(
        target: "nebula::api::webhook::bootstrap_b",
        loaded = report.loaded,
        skipped = report.skipped,
        total,
        source = "port_store",
        "B-world webhook bootstrap complete"
    );
    Ok(report)
}

/// Build the full set of resolved activations from the B-world store without
/// touching the transport. Used by the admin reload endpoint (E3) for atomic
/// slug map swaps.
///
/// # Errors
///
/// Returns [`BootstrapErrorB::Storage`] only when the underlying port store
/// call fails.
pub async fn collect_webhook_activations_b(
    store: &dyn WebhookActivationStore,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
) -> Result<(Vec<ResolvedActivation>, BootstrapReport), BootstrapErrorB> {
    let records = store.list_all_active().await?;
    let mut report = BootstrapReport::default();
    let mut activations: Vec<ResolvedActivation> = Vec::with_capacity(records.len());
    for record in &records {
        match resolve_one_b(record, registry, secrets, ctx_factory, spec_lookup).await {
            Ok(resolved) => {
                report.loaded += 1;
                activations.push(resolved);
            },
            Err(err) => {
                report.skipped += 1;
                tracing::warn!(
                    target: "nebula::api::webhook::bootstrap_b",
                    error = %err,
                    trigger_id = %record.trigger_id,
                    "skipping B-world activation during collect"
                );
            },
        }
    }
    Ok((activations, report))
}

async fn resolve_one_b(
    record: &PortWebhookActivationRecord,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
) -> Result<ResolvedActivation, BootstrapErrorB> {
    let spec = spec_lookup
        .lookup(&record.trigger_id)
        .await
        .map_err(|source| BootstrapErrorB::SecretResolution {
            // Reuse SecretResolution variant shape for lookup failures to keep
            // callers' match arms simple. The trigger_id is the "key" here.
            secret_id: record.trigger_id.clone(),
            source,
        })?
        .ok_or_else(|| BootstrapErrorB::MissingSpec {
            trigger_id: record.trigger_id.clone(),
        })?;

    let secret = secrets.resolve(&spec.secret_id).await.map_err(|source| {
        BootstrapErrorB::SecretResolution {
            secret_id: spec.secret_id.clone(),
            source,
        }
    })?;

    let action_spec = into_action_spec(&spec, secret);

    let factory = registry
        .lookup_webhook_factory(&spec.action_kind)
        .ok_or_else(|| BootstrapErrorB::UnknownProvider(spec.action_kind.clone()))?;

    let BuiltWebhookHandler { handler, config } =
        factory
            .build(&action_spec)
            .map_err(|source| BootstrapErrorB::Factory {
                kind: spec.action_kind.clone(),
                source,
            })?;

    let coords = TriggerCoordinates::new(&record.slug, "", "");
    let ctx = ctx_factory.build(record);
    Ok((coords, handler, config, ctx))
}

async fn register_one_b(
    record: &PortWebhookActivationRecord,
    registry: &ActionRegistry,
    transport: &WebhookTransport,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
) -> Result<(), BootstrapErrorB> {
    let (coords, handler, config, ctx) =
        resolve_one_b(record, registry, secrets, ctx_factory, spec_lookup).await?;

    let action_kind = {
        // Re-fetch spec for the kind label (already verified to exist in resolve_one_b).
        spec_lookup
            .lookup(&record.trigger_id)
            .await
            .ok()
            .flatten()
            .map(|s| s.action_kind)
            .unwrap_or_default()
    };

    register_with_transport(transport, coords.clone(), handler, config, ctx).map_err(|err| {
        match err {
            ActivationError::DuplicateRegistration => {
                BootstrapErrorB::DuplicateRegistration { coords }
            },
            other => BootstrapErrorB::Factory {
                kind: action_kind,
                source: FactoryError::InvalidSpec {
                    kind: "transport",
                    reason: other.to_string(),
                },
            },
        }
    })?;

    tracing::debug!(
        target: "nebula::api::webhook::bootstrap_b",
        trigger_id = %record.trigger_id,
        scope = ?record.scope,
        "B-world webhook activation registered"
    );
    Ok(())
}

fn bootstrap_failure_reason_b(err: &BootstrapErrorB) -> &'static str {
    match err {
        BootstrapErrorB::Storage(_) => webhook_bootstrap_failure_reason::STORAGE,
        BootstrapErrorB::MissingSpec { .. }
        | BootstrapErrorB::SecretResolution { .. }
        | BootstrapErrorB::UnknownProvider(_)
        | BootstrapErrorB::Factory { .. }
        | BootstrapErrorB::DuplicateRegistration { .. } => {
            webhook_bootstrap_failure_reason::FACTORY
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use nebula_action::webhook::providers;
    use nebula_engine::ActionRegistry;
    use nebula_storage::repos::InMemoryWebhookActivationRepo;
    use nebula_storage::rows::{
        WebhookActivationCoords, WebhookActivationRecord, WebhookActivationSpec,
    };

    use crate::transport::webhook::{WebhookTransport, WebhookTransportConfig};

    struct StaticSecretResolver {
        map: HashMap<String, Vec<u8>>,
    }

    #[async_trait]
    impl WebhookSecretResolver for StaticSecretResolver {
        async fn resolve(&self, secret_id: &str) -> Result<Vec<u8>, SecretResolutionError> {
            self.map.get(secret_id).cloned().ok_or_else(|| {
                Box::<dyn std::error::Error + Send + Sync>::from(format!(
                    "no secret for id={secret_id}"
                ))
            })
        }
    }

    struct StubCtxFactory;

    impl WebhookContextFactory for StubCtxFactory {
        fn build(&self, _record: &WebhookActivationRecord) -> TriggerRuntimeContext {
            use tokio_util::sync::CancellationToken;
            TriggerRuntimeContext::new(
                Arc::new(
                    nebula_core::BaseContext::builder()
                        .cancellation(CancellationToken::new())
                        .build(),
                ),
                nebula_core::WorkflowId::new(),
                nebula_core::node_key!("storage_bootstrap"),
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

    fn registry_with_default_factories() -> ActionRegistry {
        let registry = ActionRegistry::new();
        for factory in providers::default_factories() {
            registry.register_webhook_provider(factory);
        }
        registry
    }

    fn transport() -> WebhookTransport {
        WebhookTransport::new(WebhookTransportConfig::default())
    }

    fn secrets(pairs: &[(&str, &[u8])]) -> StaticSecretResolver {
        let mut map = HashMap::new();
        for (id, bytes) in pairs {
            map.insert((*id).to_string(), bytes.to_vec());
        }
        StaticSecretResolver { map }
    }

    #[tokio::test]
    async fn empty_repo_loads_zero_activations() {
        let repo = InMemoryWebhookActivationRepo::new();
        let registry = registry_with_default_factories();
        let report = bootstrap_webhook_activations(
            &repo,
            &registry,
            &transport(),
            &secrets(&[]),
            &StubCtxFactory,
            None,
        )
        .await
        .unwrap();
        assert_eq!(report.loaded, 0);
        assert_eq!(report.skipped, 0);
    }

    #[tokio::test]
    async fn loads_generic_activation_end_to_end() {
        let repo = InMemoryWebhookActivationRepo::with_records(vec![record(
            "stripe-prod",
            "generic",
            "cred_x",
        )]);
        let registry = registry_with_default_factories();
        let report = bootstrap_webhook_activations(
            &repo,
            &registry,
            &transport(),
            &secrets(&[("cred_x", b"super-secret-key")]),
            &StubCtxFactory,
            None,
        )
        .await
        .unwrap();
        assert_eq!(report.loaded, 1, "exactly one activation must be loaded");
        assert_eq!(report.skipped, 0);
    }

    #[tokio::test]
    async fn unknown_provider_kind_is_skipped() {
        let repo = InMemoryWebhookActivationRepo::with_records(vec![record(
            "weird",
            "no-such-provider",
            "cred_x",
        )]);
        let registry = registry_with_default_factories();
        let report = bootstrap_webhook_activations(
            &repo,
            &registry,
            &transport(),
            &secrets(&[("cred_x", b"k")]),
            &StubCtxFactory,
            None,
        )
        .await
        .unwrap();
        assert_eq!(report.loaded, 0);
        assert_eq!(report.skipped, 1);
    }

    #[tokio::test]
    async fn missing_secret_is_skipped() {
        let repo = InMemoryWebhookActivationRepo::with_records(vec![record(
            "stripe-prod",
            "generic",
            "cred_missing",
        )]);
        let registry = registry_with_default_factories();
        let report = bootstrap_webhook_activations(
            &repo,
            &registry,
            &transport(),
            &secrets(&[]),
            &StubCtxFactory,
            None,
        )
        .await
        .unwrap();
        assert_eq!(report.loaded, 0);
        assert_eq!(report.skipped, 1);
    }

    /// Storage failures bubble out — the composition root decides
    /// whether to log + degrade or panic.
    #[tokio::test]
    async fn storage_error_propagates() {
        struct FailingRepo;
        #[async_trait]
        impl WebhookActivationRepo for FailingRepo {
            async fn list_active(&self) -> Result<Vec<WebhookActivationRecord>, StorageError> {
                Err(StorageError::Connection("boom".into()))
            }
            async fn find_by_webhook_path(
                &self,
                _: &str,
            ) -> Result<Option<WebhookActivationRecord>, StorageError> {
                Ok(None)
            }
        }
        let registry = registry_with_default_factories();
        let err = bootstrap_webhook_activations(
            &FailingRepo,
            &registry,
            &transport(),
            &secrets(&[]),
            &StubCtxFactory,
            None,
        )
        .await
        .expect_err("must surface storage error");
        assert!(matches!(err, BootstrapError::Storage(_)), "got {err:?}");
    }

    /// `WebhookContextFactory` may be a closure-flavoured
    /// implementation — keep the trait usable from a one-liner test.
    #[allow(dead_code)]
    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn marker_traits_are_satisfied() {
        assert_send_sync::<BootstrapReport>();
        let _ = Mutex::new(BootstrapReport::default());
    }
}
