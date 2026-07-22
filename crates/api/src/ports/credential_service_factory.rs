//! Test-only credential service fixtures.
//!
//! This module is compiled only for crate tests or the unsupported `test-util`
//! feature. It deliberately contains no production key policy, database-path
//! selection, or first-party process composition: those decisions live in
//! `apps/server`. The fixtures still exercise the real SQLite CAS adapter and
//! the same encryption/audit/resolver stack over an isolated in-memory
//! database.

use std::sync::Arc;

use nebula_credential::provider::ExternalProvider;
use nebula_credential::runtime::LeaseLifecycleConfig;
use nebula_credential::{
    ApiKeyCredential, BasicAuthCredential, ErasedPendingStore, SigningKeyCredential,
};
use nebula_credential::{
    CredentialRegistry, CredentialService, CredentialServiceError, DispatchError, DispatchOps,
    NoopObserver, register_runtime_ops,
};

use super::credential_builder::CredentialServiceBuilder;
use nebula_storage::credential::{
    AuditEvent, AuditSink, InMemoryPendingStore, KeyProvider, SqliteCredentialPersistence,
};
use nebula_storage_port::{CredentialPersistence, CredentialPersistenceError};

/// Audit sink that records every credential operation to the tracing log
/// (metadata only — [`AuditEvent`] carries no secret material by design).
/// Honest local-first sink: the audit trail goes to the structured log
/// stream, not silently dropped. A durable sink (DB) is a future swap;
/// this keeps the §14 audit trail visible without a backend.
struct TracingAuditSink;

impl AuditSink for TracingAuditSink {
    fn record(&self, event: &AuditEvent) -> Result<(), CredentialPersistenceError> {
        tracing::info!(
            target: "nebula.credential.audit",
            cred_id = %event.credential_id,
            op = ?event.operation,
            result = ?event.result,
            "credential audit event"
        );
        Ok(())
    }
}

/// Construction failure for a credential service test fixture.
///
/// Each variant names the composition step that failed. Source-chained
/// (`#[source]`/`#[from]`) where the underlying type is reachable from the
/// test surface.
#[derive(Debug, thiserror::Error)]
pub enum CredentialServiceFactoryError {
    /// A first-party credential KEY failed to register in the shared
    /// registry (a composition bug — first-party KEYs are statically
    /// unique, so this is unreachable in practice).
    #[error("credential registry registration failed")]
    Registry(#[from] nebula_credential::RegisterError),
    /// A capability dispatch op failed to register (e.g. a duplicate KEY
    /// across two registrars).
    #[error("credential dispatch-ops registration failed")]
    Dispatch(#[from] DispatchError),
    /// The service builder rejected the composed parts — most often a
    /// registry capability advertised without a matching registered op.
    #[error("credential service build failed")]
    Build(#[from] CredentialServiceError),
    /// The isolated SQLite test store could not be opened or migrated.
    #[error("credential store init failed: {0}")]
    Store(String),
}

/// Build a [`CredentialService`] over a **unique in-memory SQLite database**
/// (`SqliteCredentialPersistence::connect_memory`) with a caller-supplied key
/// provider — the test / throwaway-dev fixture.
///
/// The backend is the same durable adapter production uses, bound to an
/// ephemeral in-memory database that evaporates when the store is dropped, so
/// tests exercise the real CAS + encryption path without touching disk.
/// Production composition is intentionally unavailable from this module.
/// Delegates to [`with_store`].
///
/// Gated by `cfg(test)` / the `test-util` feature and not enabled by the
/// first-party release composition; unsupported for production (ADR-0023).
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if the in-memory store cannot be
/// opened/migrated, or if registry registration, dispatch-ops registration, or
/// the final service build fails.
#[cfg(any(test, feature = "test-util"))]
pub async fn with_memory_store(
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .map_err(|e| CredentialServiceFactoryError::Store(e.to_string()))?;
    with_store(store, key_provider)
}

/// Compose a [`CredentialService`] over an arbitrary `raw_store` backend with a
/// caller-supplied [`KeyProvider`].
///
/// The ordinary test path (`with_memory_store`) passes an ephemeral in-memory
/// SQLite adapter. The pending-state store is **always** the ephemeral
/// in-memory `InMemoryPendingStore` (typed universal acquisition state,
/// TTL ≤ 10 min; durable multi-replica pending is a 1.1 concern, ADR-0084).
/// Registers the first-party type set (shared with the schema port via
/// `credential_schema_registry::default_registry`) and the matching dispatch
/// ops; the advertised capabilities MUST match the ops table.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if registry registration,
/// dispatch-ops registration, or the final service build fails.
pub fn with_store<S: CredentialPersistence + 'static>(
    raw_store: S,
    key_provider: Arc<dyn KeyProvider>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    let registry = super::credential_schema_registry::default_registry()?;

    // Dispatch ops, fixed to the erased pending store so the runtime resolver
    // and `DispatchOps` need no further monomorphization. Every default type
    // is static and receives base ops only. This set MUST match the registry's
    // advertised caps or `build()` returns `CapabilityWithoutOps`.
    let mut ops = DispatchOps::<ErasedPendingStore>::new();
    register_runtime_ops::<ApiKeyCredential, ErasedPendingStore>(&mut ops)?;
    register_runtime_ops::<BasicAuthCredential, ErasedPendingStore>(&mut ops)?;
    // signing_key: static non-interactive credential (HMAC webhook secret).
    // No capability ops beyond base runtime ops — it carries no
    // INTERACTIVE/REFRESHABLE/REVOCABLE/TESTABLE caps in the registry.
    register_runtime_ops::<SigningKeyCredential, ErasedPendingStore>(&mut ops)?;

    compose_credential_service(raw_store, key_provider, registry, ops, None)
}

/// Compose a [`CredentialService`] over `raw_store` with a **caller-supplied
/// registry + dispatch ops**, wrapping the shared secure stack (audit /
/// encryption / in-memory pending / lease lifecycle). The registry's advertised
/// capabilities MUST match the ops table or [`CredentialServiceBuilder::build`]
/// returns [`CredentialServiceError::CapabilityWithoutOps`].
///
/// Both [`with_store`] (first-party set) and the test factory variants funnel
/// through here so the secure-stack composition lives in exactly one place.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError::Build`] if the builder rejects the
/// composed parts (capability/ops mismatch).
fn compose_credential_service<S: CredentialPersistence + 'static>(
    raw_store: S,
    key_provider: Arc<dyn KeyProvider>,
    registry: CredentialRegistry,
    ops: DispatchOps<ErasedPendingStore>,
    external_provider: Option<Arc<dyn ExternalProvider>>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    tracing::warn!(
        "credential: audit sink is log-only (target=nebula.credential.audit); \
         the audit trail is NOT durably persisted to a backend yet."
    );
    let audit_sink: Arc<dyn AuditSink> = Arc::new(TracingAuditSink);

    let pending = ErasedPendingStore::new(Arc::new(InMemoryPendingStore::new()));
    let observer = Arc::new(NoopObserver::new());
    let lease_config = LeaseLifecycleConfig::default();
    // Process-wide shutdown token for the lease reaper. The token is owned by
    // the service; the lease task stops when the service (and so this token)
    // is dropped at process exit — `apps/server` carries no `tokio-util` dep,
    // so the factory mints the token internally.
    let shutdown = tokio_util::sync::CancellationToken::new();

    let mut builder = CredentialServiceBuilder::new(
        raw_store,
        key_provider,
        audit_sink,
        pending,
        Arc::new(registry),
        Arc::new(ops),
        observer,
        lease_config,
        shutdown,
    );
    if let Some(provider) = external_provider {
        // External (unwired) source: the built service rejects resolution with
        // `ExternalSourceNotWired` (the resolution bridge, ADR-0051, is not yet
        // built) — `from_secure_parts` gates the resolver from this source.
        builder = builder.external_providers(provider);
    }
    let service = builder.build()?;

    tracing::info!("credential: CredentialService composed (encrypted-at-rest)");
    Ok(Arc::new(service))
}

/// Build a [`CredentialService`] over a unique in-memory SQLite database with a
/// **caller-supplied registry + dispatch ops** — the test fixture for exercising
/// the facade against credential types the first-party set lacks (e.g. a
/// non-interactive *and* Revocable type; every default type is static and
/// advertises no lifecycle capability).
///
/// Gated by `cfg(test)` / the `test-util` feature and not enabled by the
/// first-party release composition; unsupported for production (ADR-0023).
/// Mirrors [`with_memory_store`] but takes the registry/ops the caller composed
/// instead of the first-party set.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if the in-memory store cannot be
/// opened/migrated or the final service build fails (capability/ops mismatch).
#[cfg(any(test, feature = "test-util"))]
pub async fn with_memory_store_parts(
    key_provider: Arc<dyn KeyProvider>,
    registry: CredentialRegistry,
    ops: DispatchOps<ErasedPendingStore>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .map_err(|e| CredentialServiceFactoryError::Store(e.to_string()))?;
    compose_credential_service(store, key_provider, registry, ops, None)
}

/// Build a [`CredentialService`] over an in-memory store but with an **external
/// `StateSource`** backed by `provider`, whose resolution bridge (ADR-0051) is
/// not yet wired — every resolution path then fails closed with
/// `ExternalSourceNotWired`. The test fixture for the wrong-source guard.
///
/// Gated by `cfg(test)` / the `test-util` feature and not enabled by the
/// first-party release composition; unsupported for production.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if the in-memory store cannot be
/// opened/migrated or the final service build fails.
#[cfg(any(test, feature = "test-util"))]
pub async fn with_memory_store_external(
    key_provider: Arc<dyn KeyProvider>,
    registry: CredentialRegistry,
    ops: DispatchOps<ErasedPendingStore>,
    provider: Arc<dyn ExternalProvider>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    let store = SqliteCredentialPersistence::connect_memory()
        .await
        .map_err(|e| CredentialServiceFactoryError::Store(e.to_string()))?;
    compose_credential_service(store, key_provider, registry, ops, Some(provider))
}
