//! Test-only credential service fixtures.
//!
//! This module is compiled only for crate tests or the unsupported `test-util`
//! feature. It deliberately contains no production key policy, database-path
//! selection, or first-party process composition: those decisions live in
//! `apps/server`. The fixtures still exercise the real SQLite CAS adapter and
//! the same encryption/audit/resolver stack over an isolated in-memory
//! database.

use std::{future::Future, pin::Pin, sync::Arc};

use nebula_credential::provider::ExternalProvider;
use nebula_credential::runtime::{
    AcquisitionTransport, AcquisitionTransportError, LeaseLifecycleConfig, RefreshTransport,
    RefreshTransportError, TokenPostRequest, TokenPostResponse,
};
use nebula_credential::{
    ApiKeyCredential, BasicAuthCredential, ErasedPendingStore, OAuth2Credential,
    SigningKeyCredential,
};
use nebula_credential::{
    CredentialRegistry, CredentialService, CredentialServiceError, DispatchError, DispatchOps,
    NoopObserver, register_interactive_ops, register_refreshable_ops, register_runtime_ops,
};

use super::credential_builder::CredentialServiceBuilder;
use nebula_storage::credential::{
    AuditEvent, AuditSink, InMemoryPendingStore, KeyProvider, SqliteCredentialPersistence,
};
use nebula_storage::sqlite::SqliteTenantProvisioningStore;
use nebula_storage_port::{
    CredentialCommit, CredentialCreate, CredentialMaterialEpoch, CredentialOperationStatus,
    CredentialPersistence, CredentialPersistenceError, CredentialReplacement, CredentialSelector,
    CredentialTombstone, RefreshClaimStore, RefreshRetrySnapshot, StoredCredential,
    StoredCredentialHead, StoredCredentialOperationalHead,
};

/// Test fixture: a credential store that **auto-provisions** the workspace
/// of every credential it creates.
///
/// Credentials belong to a live workspace in the deployment database
/// (migration 0070). An isolated test database holds no tenants, while the
/// fixtures resolve workspaces through their own (in-memory) tenant
/// directory, so this decorator provisions the tenant a create names — the
/// org with that workspace as its default — on the same deployment database
/// before delegating. Every other operation passes through unchanged.
///
/// Because it provisions on demand, a test asserting missing-workspace or
/// archived-workspace behaviour must **not** use it: it would hide exactly
/// the refusal under test. Use the raw store and a tenant provisioning store
/// on the shared pool instead.
#[derive(Debug, Clone)]
pub struct TenantProvisionedStore {
    inner: SqliteCredentialPersistence,
    tenants: SqliteTenantProvisioningStore,
}

impl TenantProvisionedStore {
    /// Wrap `inner`, provisioning tenants through `tenants`, which must sit
    /// on the same deployment database.
    #[must_use]
    pub fn new(inner: SqliteCredentialPersistence, tenants: SqliteTenantProvisioningStore) -> Self {
        Self { inner, tenants }
    }

    /// A store on a fresh, isolated in-memory deployment database.
    ///
    /// # Errors
    ///
    /// [`CredentialPersistenceError::Unavailable`] when the database cannot
    /// open or migrate.
    pub async fn memory() -> Result<Self, CredentialPersistenceError> {
        let pool = nebula_storage::sqlite::open_memory_deployment()
            .await
            .map_err(|_| CredentialPersistenceError::Unavailable)?;
        let inner = SqliteCredentialPersistence::connect_pool(pool.clone())
            .await
            .map_err(|_| CredentialPersistenceError::Unavailable)?;
        Ok(Self::new(inner, SqliteTenantProvisioningStore::new(pool)))
    }

    /// The wrapped store, for its claim, pending and schedule adapters.
    #[must_use]
    pub fn inner(&self) -> &SqliteCredentialPersistence {
        &self.inner
    }

    /// Provision the tenant `owner` names in this store's database.
    ///
    /// # Errors
    ///
    /// [`CredentialPersistenceError::Unavailable`] when provisioning fails.
    pub async fn provision(
        &self,
        owner: &nebula_storage_port::CredentialOwner,
    ) -> Result<(), CredentialPersistenceError> {
        provision_owner(&self.tenants, owner).await
    }
}

/// Test fixture: provision the tenant `owner` names — its org, with its
/// workspace as the default one — through `tenants`. Idempotent.
///
/// # Errors
///
/// [`CredentialPersistenceError::Unavailable`] when provisioning fails.
pub async fn provision_owner(
    tenants: &dyn nebula_storage_port::store::TenantProvisioningStore,
    owner: &nebula_storage_port::CredentialOwner,
) -> Result<(), CredentialPersistenceError> {
    use nebula_storage_port::dto::{
        PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningRequest,
    };

    let Some(scope) = owner.scope() else {
        // A partition naming no workspace owns nothing; the store answers.
        return Ok(());
    };
    fn unavailable<E>(_: E) -> CredentialPersistenceError {
        CredentialPersistenceError::Unavailable
    }
    let request = TenantProvisioningRequest::new(
        TenantOrgCreate::new(
            scope.org_id.clone(),
            scope.org_id.clone(),
            "Fixture".into(),
            "fixture".into(),
            "free".into(),
            None,
            serde_json::json!({}),
        )
        .map_err(unavailable)?,
        TenantDefaultWorkspaceCreate::new(
            scope.workspace_id.clone(),
            "default".into(),
            "Default".into(),
            None,
            "fixture".into(),
            serde_json::json!({}),
        )
        .map_err(unavailable)?,
        PrincipalKind::User,
        "fixture-owner".into(),
        None,
    )
    .map_err(unavailable)?;
    tenants
        .provision_tenant(request)
        .await
        .map_err(unavailable)?;
    Ok(())
}

#[async_trait::async_trait]
impl CredentialPersistence for TenantProvisionedStore {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        self.inner.get(selector).await
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        self.inner.get_head(selector).await
    }

    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        self.inner.operation_status(selector).await
    }

    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        self.inner.get_operational_head(selector).await
    }

    async fn get_with_operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(StoredCredential, Option<CredentialOperationStatus>), CredentialPersistenceError>
    {
        self.inner.get_with_operation_status(selector).await
    }

    async fn list_operational_heads(
        &self,
        owner: &nebula_storage_port::CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialOperationalHead>, CredentialPersistenceError> {
        self.inner.list_operational_heads(owner, state_kind).await
    }

    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        self.inner.refresh_retry_snapshot(selector).await
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.provision(selector.owner()).await?;
        self.inner.create(selector, create).await
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner.replace(selector, replacement).await
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner.tombstone(selector, tombstone).await
    }

    async fn tombstone_revoked_material(
        &self,
        selector: &CredentialSelector,
        expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        self.inner
            .tombstone_revoked_material(selector, expected_material_epoch)
            .await
    }

    async fn list(
        &self,
        owner: &nebula_storage_port::CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<nebula_core::CredentialId>, CredentialPersistenceError> {
        self.inner.list(owner, state_kind).await
    }

    async fn list_heads(
        &self,
        owner: &nebula_storage_port::CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        self.inner.list_heads(owner, state_kind).await
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        self.inner.exists(selector).await
    }
}

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

/// Deterministic no-network refresh transport for API-only fixtures.
///
/// Authorization-code initiation remains local and can therefore exercise the
/// universal pending protocol. Any token exchange fails closed. Tests of real
/// HTTP policy belong to the first-party composition root in `apps/server`;
/// this collaborator prevents the API fixture from silently acquiring proxy,
/// redirect, retry, or DNS behavior.
#[derive(Debug)]
struct NoNetworkRefreshTransport;

impl RefreshTransport for NoNetworkRefreshTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>>
    {
        Box::pin(async { Err(RefreshTransportError::Send) })
    }
}

impl AcquisitionTransport for NoNetworkRefreshTransport {
    fn post_token<'a>(
        &'a self,
        _request: TokenPostRequest,
    ) -> Pin<
        Box<dyn Future<Output = Result<TokenPostResponse, AcquisitionTransportError>> + Send + 'a>,
    > {
        Box::pin(async { Err(AcquisitionTransportError::Send) })
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

/// Build a [`CredentialService`] over a **unique in-memory SQLite deployment
/// database** ([`TenantProvisionedStore::memory`]) with a caller-supplied key
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
    with_store(memory_store().await?, key_provider)
}

/// A [`TenantProvisionedStore`] on a fresh in-memory deployment database.
#[cfg(any(test, feature = "test-util"))]
async fn memory_store() -> Result<TenantProvisionedStore, CredentialServiceFactoryError> {
    TenantProvisionedStore::memory()
        .await
        .map_err(|e| CredentialServiceFactoryError::Store(e.to_string()))
}

/// Compose a [`CredentialService`] over an admitted SQLite test backend with a
/// caller-supplied [`KeyProvider`] and claims from that same database.
///
/// The ordinary test path (`with_memory_store`) passes an ephemeral in-memory
/// SQLite adapter. The store auto-provisions workspaces (see
/// [`TenantProvisionedStore`]). This API-only fixture deliberately keeps pending state in
/// `InMemoryPendingStore`; the supported server composition injects the
/// admitted backend's encrypted durable pending-state adapter.
/// Registers the first-party type set (shared with the schema port via
/// `credential_schema_registry::default_registry`) and the matching dispatch
/// ops; the advertised capabilities MUST match the ops table.
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if registry registration,
/// dispatch-ops registration, or the final service build fails.
pub fn with_store(
    raw_store: TenantProvisionedStore,
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
    register_runtime_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_interactive_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    register_refreshable_ops::<OAuth2Credential, ErasedPendingStore>(&mut ops)?;
    // signing_key: static non-interactive credential (HMAC webhook secret).
    // No capability ops beyond base runtime ops — it carries no
    // INTERACTIVE/REFRESHABLE/REVOCABLE/TESTABLE caps in the registry.
    register_runtime_ops::<SigningKeyCredential, ErasedPendingStore>(&mut ops)?;

    let claims = Arc::new(raw_store.inner().refresh_claim_repo());
    compose_credential_service(raw_store, claims, key_provider, registry, ops, None)
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
    claims: Arc<dyn RefreshClaimStore>,
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

    let transport = Arc::new(NoNetworkRefreshTransport);
    let mut builder = CredentialServiceBuilder::new(
        raw_store,
        claims,
        key_provider,
        audit_sink,
        pending,
        Arc::new(registry),
        Arc::new(ops),
        transport.clone(),
        transport,
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
    let store = memory_store().await?;
    let claims = Arc::new(store.inner().refresh_claim_repo());
    compose_credential_service(store, claims, key_provider, registry, ops, None)
}

/// Like [`with_memory_store_parts`], but the refresh-claim store is
/// **caller-supplied** instead of derived from the SQLite store, so a test can
/// hold the same claim handle the service's coordinator reads — to observe or
/// poison claims, or to hand the object to the reconciliation controller as its
/// adjudicator — without hand-composing the secure stack.
///
/// Gated by `cfg(test)` / the `test-util` feature and not enabled by the
/// first-party release composition; unsupported for production (ADR-0023).
///
/// # Errors
///
/// Returns [`CredentialServiceFactoryError`] if the in-memory store cannot be
/// opened/migrated or the final service build fails (capability/ops mismatch).
#[cfg(any(test, feature = "test-util"))]
pub async fn with_memory_store_and_claims(
    key_provider: Arc<dyn KeyProvider>,
    registry: CredentialRegistry,
    ops: DispatchOps<ErasedPendingStore>,
    claims: Arc<dyn RefreshClaimStore>,
) -> Result<Arc<CredentialService>, CredentialServiceFactoryError> {
    compose_credential_service(
        memory_store().await?,
        claims,
        key_provider,
        registry,
        ops,
        None,
    )
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
    let store = memory_store().await?;
    let claims = Arc::new(store.inner().refresh_claim_repo());
    compose_credential_service(store, claims, key_provider, registry, ops, Some(provider))
}
