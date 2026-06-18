//! Storage-driven webhook bootstrap (webhook activation — ADR-0096, B-world).
//!
//! Reads active webhook activations from the spec-16 port store
//! (`WebhookActivationStore`) and validates each row can be built into a
//! handler. Routing of B-world activations onto the in-memory map for
//! live dispatch is deferred to U-D1.4b (durable emitter install); until
//! then the port store is the authority and `resolve_by_token` confirms
//! token identity on the hot path.
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

use std::future::Future;

use async_trait::async_trait;
use nebula_action::{
    BuiltWebhookHandler, FactoryError, TriggerRuntimeContext,
    webhook::factory::WebhookActivationSpec as ActionWebhookActivationSpec,
};
use nebula_engine::ActionRegistry;
use nebula_metrics::{
    MetricsRegistry, NEBULA_WEBHOOK_BOOTSTRAP_FAILURES_TOTAL, webhook_bootstrap_failure_reason,
};
use nebula_storage::rows::{
    WebhookActivationSpec as StorageWebhookActivationSpec, WebhookTimestampFormat,
};
use nebula_storage_port::store::WebhookActivationStore;
use nebula_storage_port::{
    Scope, StorageError as PortStorageError,
    dto::WebhookActivationRecord as PortWebhookActivationRecord,
};
use thiserror::Error;

/// Boxed error type returned by [`WebhookSecretResolver::resolve`].
/// The bootstrap wraps it in [`BootstrapError::SecretResolution`]
/// alongside the failing `secret_id`.
pub type SecretResolutionError = Box<dyn std::error::Error + Send + Sync>;

/// Resolves a credential identifier (storage-layer string) to the
/// raw HMAC secret bytes consumed by the factory.
///
/// Production deployments wire this through [`nebula_credential::CredentialService`]
/// via [`crate::transport::webhook::secret_resolver::CredentialBackedWebhookSecretResolver`];
/// tests wire an in-memory map. Returning an empty `Vec` is **not** allowed —
/// the factory would then build a fail-closed handler.  Surface the
/// misconfiguration as a typed error inside the boxed return.
///
/// # Tenant isolation
///
/// `scope` is mandatory: [`nebula_credential::CredentialService::resolve_for_slot`]
/// requires a [`nebula_credential::TenantScope`] derived from the B-world
/// activation row's `scope`.  Without it the credential layer cannot enforce
/// the owner check.
#[async_trait]
pub trait WebhookSecretResolver: Send + Sync {
    /// Resolve `secret_id` to raw secret bytes, scoped to `scope`.
    ///
    /// # Errors
    ///
    /// Returns a boxed error on lookup failure. The bootstrap wraps
    /// it in [`BootstrapError::SecretResolution`].
    async fn resolve(
        &self,
        scope: &Scope,
        secret_id: &str,
    ) -> Result<Vec<u8>, SecretResolutionError>;
}

/// Result of [`bootstrap_webhook_activations`].
///
/// `loaded` is the count of activation rows that were validated
/// successfully. `skipped` is the count of rows that surfaced a
/// non-storage error (decode mismatch, factory rejection, secret
/// resolution failure).
#[derive(Debug, Clone, Copy, Default)]
pub struct BootstrapReport {
    /// Activations validated from the port store.
    pub loaded: usize,
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

/// Walk the B-world port store's active webhook activations and validate each
/// can be built into a handler. Returns a [`BootstrapReport`] with loaded/skipped
/// counts.
///
/// Routing of B-world activations onto the in-memory dispatch map is deferred to
/// U-D1.4b (durable emitter install). This function validates the port store is
/// accessible and each row's factory + secret can be resolved, so startup logs
/// surface misconfiguration early.
///
/// Failure isolation is per-row: per-row failures are logged and counted;
/// storage failures bubble out.
///
/// # Errors
///
/// Returns [`BootstrapError::Storage`] only when the underlying port store
/// call fails. Per-row failures are absorbed into `report.skipped`.
pub async fn bootstrap_webhook_activations(
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
        match validate_one(record, registry, secrets, ctx_factory, spec_lookup).await {
            Ok(()) => report.loaded += 1,
            Err(err) => {
                report.skipped += 1;
                record_bootstrap_failure(metrics, bootstrap_failure_reason(&err));
                tracing::warn!(
                    target: "nebula::api::webhook::bootstrap",
                    error = %err,
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

    tracing::info!(
        target: "nebula::api::webhook::bootstrap",
        loaded = report.loaded,
        skipped = report.skipped,
        total,
        source = "port_store",
        "webhook bootstrap complete (dispatch routing deferred to U-D1.4b)"
    );
    Ok(report)
}

/// Validate one B-world activation row: spec lookup + secret resolution +
/// factory build. Does not touch the transport routing map.
async fn validate_one(
    record: &PortWebhookActivationRecord,
    registry: &ActionRegistry,
    secrets: &dyn WebhookSecretResolver,
    ctx_factory: &dyn WebhookActivationContextFactory,
    spec_lookup: &dyn TriggerSpecLookup,
) -> Result<(), BootstrapError> {
    let spec = spec_lookup
        .lookup(&record.scope, &record.trigger_id)
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
        .map_err(|source| BootstrapError::SecretResolution {
            secret_id: spec.secret_id.clone(),
            source,
        })?;

    let action_spec = into_action_spec(&spec, secret);

    let factory = registry
        .lookup_webhook_factory(&spec.provider)
        .ok_or_else(|| BootstrapError::UnknownProvider(spec.provider.clone()))?;

    // Build the handler to prove the factory accepts the spec. The
    // resulting handler + ctx are discarded — dispatch onto the in-memory
    // routing map is U-D1.4b.
    let BuiltWebhookHandler { .. } =
        factory
            .build(&action_spec)
            .map_err(|source| BootstrapError::Factory {
                kind: spec.provider.clone(),
                source,
            })?;
    let _ctx = ctx_factory.build(record);

    tracing::debug!(
        target: "nebula::api::webhook::bootstrap",
        trigger_id = %record.trigger_id,
        scope = ?record.scope,
        "webhook activation validated (dispatch deferred to U-D1.4b)"
    );
    Ok(())
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
