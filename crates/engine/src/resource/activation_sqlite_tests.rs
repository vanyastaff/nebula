//! Storage-backed acceptance of the credential use revision.
//!
//! The real read-only credential runtime (`CredentialProjectionRuntime`)
//! over encrypted SQLite persistence and the real SQLite refresh-claim
//! repository: the admission epoch activation compares is the one the
//! backend writes, not a scripted value. A revoke claim acquired and then
//! abandoned closes use by itself (the claim bumps the epoch) and reopens by
//! expiry alone, with no write in between.
//!
//! The strict manager's admission is accepted per acquire (S1–S5) and, for
//! the managed call facade, per attempt (F1–F6): each provider attempt reads
//! the credential's availability through the real runtime, and a change
//! landing between two attempts of one unit refuses the later one. F7 takes
//! the action path: the engine's resource accessor and
//! `ActionContextExt::resource_handle_by_id`.
use std::{num::NonZeroU32, str::FromStr, time::Duration};

use nebula_credential::{
    BearerTokenCredential, Credential, CredentialProjectionRuntime, CredentialRegistry,
    CredentialState, DispatchOps, ErasedPendingStore, SecretString, StateSource,
    register_runtime_ops, scheme::SecretToken,
};
use nebula_resource::call::{
    Cost, Effect, Lease, Operation, OperationCx, OperationError, PinSlots, SentState,
};
use nebula_storage::credential::{
    EncryptionLayer, EnvKeyProvider, SqliteCredentialPersistence, SqliteRefreshClaimRepo,
};
use nebula_storage_port::{
    CredentialCreate, CredentialMaterialEpoch, CredentialMaterialTransition, CredentialOwner,
    CredentialPersistence, CredentialPersistenceError, CredentialReplacement, CredentialSelector,
    RefreshRetryTransition, SecretBytes,
    store::{ClaimAttempt, CredentialOperationIntent, RefreshClaimStore, ReplicaId},
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::Notify;

use super::*;

const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
const BEARER_KIND: &str = "activation.bearer";

/// One `bearer_token` credential slot, projected as the real runtime
/// projects it.
#[derive(Clone)]
struct BearerRow {
    auth: Arc<nebula_resource::SlotCell<nebula_credential::CredentialGuard<SecretToken>>>,
}

impl BearerRow {
    fn new() -> Self {
        Self {
            auth: Arc::new(nebula_resource::SlotCell::empty()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for BearerRow {
    type Config = LabelConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("activation.bearer")
    }

    async fn create(
        &self,
        _config: &LabelConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, ResourceError> {
        Ok(Arc::new(AtomicU64::new(1)))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("activation.bearer"),
            String::new(),
        )
    }
}

impl DeclaresDependencies for BearerRow {
    fn dependencies() -> Dependencies {
        Dependencies::new().slot_field(SlotField {
            slot_key: AUTH_SLOT,
            default_id: AUTH_SLOT,
            kind: SlotKind::Credential {
                type_id: std::any::TypeId::of::<BearerTokenCredential>(),
                type_name: "bearer_token",
                key: CredentialKey::new(BearerTokenCredential::KEY).expect("valid credential key"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

impl nebula_resource::HasCredentialSlots for BearerRow {
    fn credential_slot_epoch(&self) -> u64 {
        self.auth.generation()
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &[AUTH_SLOT]
    }

    fn supports_credential_slot_projection(&self, slot: &str) -> bool {
        slot == AUTH_SLOT
    }

    fn credential_slot_metadata(
        &self,
        slot: &str,
    ) -> Option<nebula_credential::CredentialGuardMetadata> {
        (slot == AUTH_SLOT)
            .then(|| self.auth.projection_metadata())
            .flatten()
    }

    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<nebula_credential::CredentialGuardMetadata>)> {
        (slot == AUTH_SLOT).then(|| self.auth.projection_snapshot())
    }

    fn install_credential_slot_at_generation(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
        expected_generation: u64,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        let (metadata, guard) = bearer(slot, guard)?;
        self.auth
            .install_projected_at_generation(expected_generation, metadata, guard)
    }

    fn fence_credential_slot_at_generation(
        &self,
        slot: &str,
        expected_generation: u64,
        fence: &mut dyn FnMut(),
    ) -> Result<(), nebula_resource::SlotInstallError> {
        if slot != AUTH_SLOT {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        self.auth
            .fence_projection_at_generation(expected_generation, fence)
    }

    fn install_credential_slot(
        &self,
        slot: &str,
        guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        let (metadata, guard) = bearer(slot, guard)?;
        self.auth.install_projected(metadata, guard)
    }

    fn revoke_credential_slot(
        &self,
        slot: &str,
    ) -> Result<nebula_resource::SlotUpdate, nebula_resource::SlotInstallError> {
        if slot != AUTH_SLOT {
            return Err(nebula_resource::SlotInstallError::UnknownSlot);
        }
        Ok(self.auth.revoke())
    }
}

fn bearer(
    slot: &str,
    guard: nebula_credential::ErasedCredentialGuard,
) -> Result<
    (
        nebula_credential::CredentialGuardMetadata,
        Arc<nebula_credential::CredentialGuard<SecretToken>>,
    ),
    nebula_resource::SlotInstallError,
> {
    if slot != AUTH_SLOT {
        return Err(nebula_resource::SlotInstallError::UnknownSlot);
    }
    let metadata = guard.metadata().clone();
    let guard = guard
        .into_typed::<SecretToken>()
        .map_err(|_| nebula_resource::SlotInstallError::CredentialTypeMismatch)?;
    Ok((metadata, Arc::new(guard)))
}

/// The unit's pin: the bearer slot's material epoch and guard.
impl PinSlots for BearerRow {
    type Pinned = Option<(u64, Arc<nebula_credential::CredentialGuard<SecretToken>>)>;

    fn pin_slots(&self) -> Self::Pinned {
        self.auth.load_material_versioned()
    }
}

impl resident::ResidentProvider for BearerRow {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

/// Stored display metadata for credential name `name`.
fn display(name: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::Map::from_iter([(
        "display".to_owned(),
        serde_json::json!({ "display_name": name }),
    )])
}

/// The real projection runtime, counting decrypting projections.
struct CountingRuntime {
    inner: Arc<CredentialProjectionRuntime>,
    projections: AtomicUsize,
}

/// The real runtime's availability observer, as a strict manager holds it,
/// counting head reads.
struct CountingObserver {
    inner: Arc<CredentialProjectionRuntime>,
    reads: AtomicUsize,
}

impl CredentialAvailabilityObserver for CountingObserver {
    fn observe_availability<'a>(
        &'a self,
        scope: &'a TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        cancel: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        nebula_credential::CredentialAvailabilityObservation,
                        CredentialObserveError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner
            .observe_availability(scope, credential_id, expected_key, cancel)
    }
}

/// Credential persistence that can be switched off, as a store outage: every
/// read and write fails `Unavailable` while it is down.
#[derive(Debug)]
struct SwitchableOutage {
    inner: Arc<dyn CredentialPersistence>,
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl SwitchableOutage {
    fn check(&self) -> Result<(), CredentialPersistenceError> {
        if self.down.load(Ordering::SeqCst) {
            Err(CredentialPersistenceError::Unavailable)
        } else {
            Ok(())
        }
    }
}

#[async_trait::async_trait]
impl CredentialPersistence for SwitchableOutage {
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::StoredCredential, CredentialPersistenceError> {
        self.check()?;
        self.inner.get(selector).await
    }

    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::StoredCredentialHead, CredentialPersistenceError> {
        self.check()?;
        self.inner.get_head(selector).await
    }

    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::store::CredentialOperationStatus, CredentialPersistenceError>
    {
        self.check()?;
        self.inner.operation_status(selector).await
    }

    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::StoredCredentialOperationalHead, CredentialPersistenceError>
    {
        self.check()?;
        self.inner.get_operational_head(selector).await
    }

    async fn get_with_operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<
        (
            nebula_storage_port::StoredCredential,
            Option<nebula_storage_port::store::CredentialOperationStatus>,
        ),
        CredentialPersistenceError,
    > {
        self.check()?;
        self.inner.get_with_operation_status(selector).await
    }

    async fn list_operational_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<nebula_storage_port::StoredCredentialOperationalHead>, CredentialPersistenceError>
    {
        self.check()?;
        self.inner.list_operational_heads(owner, state_kind).await
    }

    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<nebula_storage_port::RefreshRetrySnapshot, CredentialPersistenceError> {
        self.check()?;
        self.inner.refresh_retry_snapshot(selector).await
    }

    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<nebula_storage_port::CredentialCommit, CredentialPersistenceError> {
        self.check()?;
        self.inner.create(selector, create).await
    }

    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<nebula_storage_port::CredentialCommit, CredentialPersistenceError> {
        self.check()?;
        self.inner.replace(selector, replacement).await
    }

    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: nebula_storage_port::CredentialTombstone,
    ) -> Result<nebula_storage_port::CredentialCommit, CredentialPersistenceError> {
        self.check()?;
        self.inner.tombstone(selector, tombstone).await
    }

    async fn tombstone_revoked_material(
        &self,
        selector: &CredentialSelector,
        expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<nebula_storage_port::CredentialCommit, CredentialPersistenceError> {
        self.check()?;
        self.inner
            .tombstone_revoked_material(selector, expected_material_epoch)
            .await
    }

    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        self.check()?;
        self.inner.list(owner, state_kind).await
    }

    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<nebula_storage_port::StoredCredentialHead>, CredentialPersistenceError> {
        self.check()?;
        self.inner.list_heads(owner, state_kind).await
    }

    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        self.check()?;
        self.inner.exists(selector).await
    }
}

impl CredentialSlotResolver for CountingRuntime {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        required_capabilities: Capabilities,
        cancel: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        nebula_credential::ErasedCredentialGuard,
                        CredentialSlotResolveError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.projections.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve_slot(
            scope,
            credential_id,
            expected_key,
            required_capabilities,
            cancel,
        )
    }

    fn as_availability_observer(&self) -> Option<&dyn CredentialAvailabilityObserver> {
        self.inner.as_availability_observer()
    }
}

struct SqliteFixture {
    store: Arc<dyn CredentialPersistence>,
    claims: SqliteRefreshClaimRepo,
    sql_pool: sqlx::SqlitePool,
    resolver: CountingRuntime,
    activator: StoredResourceActivator,
    resources: Arc<InMemoryResourceStore>,
    registrars: ResourceActivatorRegistry,
    manager: Arc<Manager>,
    expr_engine: ExpressionEngine,
    scope: Scope,
    cancel: CancellationToken,
    credential_id: CredentialId,
    /// The strict manager's observer; `None` for an interim manager.
    observer: Option<Arc<CountingObserver>>,
    /// Switches the credential store off (an outage) and on.
    outage: Arc<std::sync::atomic::AtomicBool>,
    _directory: tempfile::TempDir,
}

impl SqliteFixture {
    /// An interim manager: no per-acquire availability read.
    async fn new() -> Self {
        Self::build(false).await
    }

    /// A strict manager reading through the real runtime's observer.
    async fn strict() -> Self {
        Self::build(true).await
    }

    async fn build(strict: bool) -> Self {
        let directory = tempfile::tempdir().expect("temp db directory");
        let db = directory
            .path()
            .join("activation-use-revision.sqlite")
            .to_string_lossy()
            .into_owned();
        let raw = SqliteCredentialPersistence::connect(&db)
            .await
            .expect("sqlite credential store");
        let sql_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::from_str(&db)
                    .expect("sqlite path")
                    .create_if_missing(true),
            )
            .await
            .expect("inspection pool");
        let claims = raw.refresh_claim_repo();
        let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("test key"));
        let outage = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let store: Arc<dyn CredentialPersistence> = Arc::new(SwitchableOutage {
            inner: Arc::new(EncryptionLayer::new(raw, key)),
            down: Arc::clone(&outage),
        });

        let mut registry = CredentialRegistry::new();
        registry
            .register(BearerTokenCredential, "nebula-engine-test")
            .expect("bearer registration");
        registry
            .register(nebula_credential::BasicAuthCredential, "nebula-engine-test")
            .expect("basic auth registration");
        let mut ops = DispatchOps::<ErasedPendingStore>::new();
        register_runtime_ops::<BearerTokenCredential, ErasedPendingStore>(&mut ops)
            .expect("runtime ops");
        register_runtime_ops::<nebula_credential::BasicAuthCredential, ErasedPendingStore>(
            &mut ops,
        )
        .expect("basic auth runtime ops");
        let runtime = Arc::new(
            CredentialProjectionRuntime::from_secure_parts(
                Arc::clone(&store),
                Arc::new(registry),
                Arc::new(ops),
                StateSource::LocalEncrypted,
            )
            .expect("projection runtime"),
        );
        let observer = strict.then(|| {
            Arc::new(CountingObserver {
                inner: Arc::clone(&runtime),
                reads: AtomicUsize::new(0),
            })
        });
        let manager = match &observer {
            Some(observer) => Manager::with_config(
                nebula_resource::ManagerConfig::default()
                    .with_credential_observer(
                        Arc::clone(observer) as Arc<dyn CredentialAvailabilityObserver>
                    ),
            ),
            None => Manager::new(),
        };

        let mut registrars = ResourceActivatorRegistry::new();
        registrars
            .insert(
                BEARER_KIND,
                Arc::new(KindActivator::<BearerRow, _, _>::new(
                    BearerRow::new,
                    nebula_resource::topology::fixed(|| {
                        Resident::<BearerRow>::new(resident::config::Config::default())
                    }),
                )),
            )
            .expect("bearer resource admits");

        let scope = Scope::new(
            WorkspaceId::new().to_string(),
            nebula_core::OrgId::new().to_string(),
        );
        let credential_id = CredentialId::new();
        let token = SecretToken::new(SecretString::new("sqlite-acceptance-token"));
        let data = nebula_credential::serde_secret::expose_for_serialization(|| {
            serde_json::to_vec(&token)
        })
        .expect("token encodes");
        store
            .create(
                &CredentialSelector::new(CredentialOwner::from_scope(&scope), credential_id),
                CredentialCreate::new(
                    BearerTokenCredential::KEY.to_owned(),
                    SecretBytes::new(data),
                    <SecretToken as CredentialState>::KIND.to_owned(),
                    <SecretToken as CredentialState>::VERSION,
                    Some("acceptance".to_owned()),
                    None,
                    false,
                    display("acceptance"),
                ),
            )
            .await
            .expect("credential created");

        let resources = Arc::new(InMemoryResourceStore::new());
        Self {
            store,
            claims,
            sql_pool,
            resolver: CountingRuntime {
                inner: runtime,
                projections: AtomicUsize::new(0),
            },
            activator: StoredResourceActivator::new(
                Arc::clone(&resources) as Arc<dyn ResourceStore>
            ),
            resources,
            registrars,
            manager: Arc::new(manager),
            expr_engine: ExpressionEngine::with_cache_size(16),
            scope,
            cancel: CancellationToken::new(),
            credential_id,
            observer,
            outage,
            _directory: directory,
        }
    }

    fn selector(&self) -> CredentialSelector {
        CredentialSelector::new(CredentialOwner::from_scope(&self.scope), self.credential_id)
    }

    fn projections(&self) -> usize {
        self.resolver.projections.load(Ordering::SeqCst)
    }

    async fn store_row(&self) -> (ResourceId, ResourceKey) {
        self.store_row_as(
            BEARER_KIND,
            serde_json::json!({ "label": "a" }),
            self.credential_id,
        )
        .await
    }

    /// Stores a row of resource kind `kind` whose `auth` slot is bound to
    /// `credential_id`.
    async fn store_row_as(
        &self,
        kind: &str,
        config: serde_json::Value,
        credential_id: CredentialId,
    ) -> (ResourceId, ResourceKey) {
        let resource_id = ResourceId::new();
        self.resources
            .create(
                &self.scope,
                ResourceRow {
                    id: resource_id.to_string(),
                    workspace_id: self.scope.workspace_id.clone(),
                    slug: format!("row-{resource_id}"),
                    display_name: "row".to_owned(),
                    kind: kind.to_owned(),
                    config,
                    credential_bindings: BTreeMap::from([(
                        AUTH_SLOT.to_owned(),
                        credential_id.to_string(),
                    )]),
                    topology: None,
                    resilience_override: None,
                    created_at: "2026-09-26T00:00:00Z".to_owned(),
                    created_by: "test".to_owned(),
                    version: 0,
                    deleted_at: None,
                },
            )
            .await
            .expect("row stored");
        (
            resource_id,
            ResourceKey::new(kind).expect("valid resource key"),
        )
    }

    async fn activate(
        &self,
        resource_id: ResourceId,
        key: &ResourceKey,
    ) -> Result<ActivatedResource, StoredResourceActivationError> {
        let context = ActivationContext {
            registrars: &self.registrars,
            manager: &self.manager,
            credentials: Some(&self.resolver as &dyn CredentialSlotResolver),
            expr_engine: &self.expr_engine,
            #[cfg(feature = "rotation")]
            fanout: None,
        };
        self.activator
            .activate(&context, &self.scope, resource_id, key, &self.cancel)
            .await
    }

    async fn acquire(
        &self,
        key: &ResourceKey,
        activated: &ActivatedResource,
    ) -> Result<nebula_resource::ResourceGuard<BearerRow>, ResourceError> {
        let workspace = WorkspaceId::parse(&self.scope.workspace_id).expect("workspace id");
        let ctx = ResourceContext::minimal(
            nebula_core::scope::Scope {
                workspace_id: Some(workspace),
                ..Default::default()
            },
            CancellationToken::new(),
        );
        Manager::acquire_any(
            Arc::clone(&self.manager),
            key,
            &ctx,
            &nebula_resource::AcquireOptions::default(),
            &activated.slot_identity,
        )
        .await
        .map(|lease| {
            *lease
                .downcast::<nebula_resource::ResourceGuard<BearerRow>>()
                .expect("guard type")
        })
    }

    /// A lease turned into a managed call facade.
    async fn facade(&self, key: &ResourceKey, activated: &ActivatedResource) -> Lease<BearerRow> {
        self.acquire(key, activated)
            .await
            .expect("serves")
            .into_lease()
    }

    /// `(material_epoch, admission_epoch)` the activator tracks.
    fn tracked_at(&self, resource_id: ResourceId) -> (u64, u64) {
        let slot = self
            .activator
            .rows
            .get(&(self.scope.clone(), resource_id))
            .map(|entry| Arc::clone(entry.value()))
            .expect("the row is tracked");
        let tracked = slot.try_lock().expect("no activation holds the row");
        tracked
            .active
            .as_ref()
            .expect("the row is registered")
            .bindings[0]
            .at
    }

    fn suspended(&self, activated: &ActivatedResource) -> bool {
        self.manager
            .get_row(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity,
            )
            .expect("the registration is kept")
            .credential_suspension()
            .is_some()
    }

    /// The row's gate outcome for a usable observation at `at` (the
    /// activation already admitted it when this is `NotSuspended`).
    fn gate_outcome(
        &self,
        activated: &ActivatedResource,
        at: CredentialObservedAt,
    ) -> nebula_resource::CredentialReopenOutcome {
        let ticket = self
            .manager
            .credential_gate_ticket(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity,
            )
            .expect("ticket");
        self.manager
            .reopen_credential_row(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity,
                AUTH_SLOT,
                ticket,
                at,
            )
            .expect("reopen")
    }

    /// Acquires a revoke claim and does not mark its sentinel.
    async fn claim_revoke(&self) {
        let claimed = self
            .claims
            .try_claim(
                &self.selector(),
                &ReplicaId::new("abandoning-revoker"),
                Duration::from_secs(30),
                CredentialOperationIntent::Revoke {
                    material_epoch: CredentialMaterialEpoch::MIN,
                },
            )
            .await
            .expect("the revoke claim reaches the backend");
        assert!(matches!(claimed, ClaimAttempt::Acquired(_)));
    }

    /// Lets the abandoned claim lapse: the credential reads open by clock.
    async fn lapse_claim(&self) {
        let affected = sqlx::query(
            "UPDATE credential_refresh_claims SET expires_at = 0 WHERE credential_id = ?1",
        )
        .bind(self.credential_id.to_string())
        .execute(&self.sql_pool)
        .await
        .expect("backdate the abandoned claim")
        .rows_affected();
        assert_eq!(affected, 1);
    }

    /// Availability reads the strict manager issued.
    fn reads(&self) -> usize {
        self.observer
            .as_ref()
            .expect("a strict fixture")
            .reads
            .load(Ordering::SeqCst)
    }

    /// Flags the credential for reauthentication, as the durable decision
    /// does (the backend advances the use revision).
    async fn require_reauth(&self) {
        let head = self.store.get_head(&self.selector()).await.expect("head");
        let _committed = self
            .store
            .replace(
                &self.selector(),
                CredentialReplacement::new(
                    head.version(),
                    Some("acceptance".to_owned()),
                    true,
                    display("acceptance"),
                    CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
                ),
            )
            .await
            .expect("reauthentication required");
    }

    /// Completes reauthentication with new material: the material epoch
    /// advances and the flag clears.
    async fn complete_reauth(&self) {
        let head = self.store.get_head(&self.selector()).await.expect("head");
        let token = SecretToken::new(SecretString::new("sqlite-acceptance-token-2"));
        let data = nebula_credential::serde_secret::expose_for_serialization(|| {
            serde_json::to_vec(&token)
        })
        .expect("token encodes");
        let _committed = self
            .store
            .replace(
                &self.selector(),
                CredentialReplacement::new(
                    head.version(),
                    Some("acceptance".to_owned()),
                    false,
                    display("acceptance"),
                    CredentialMaterialTransition::advance(
                        nebula_storage_port::MaterialUpdate::Replace(
                            nebula_storage_port::CredentialMaterial::new(
                                SecretBytes::new(data),
                                <SecretToken as CredentialState>::KIND.to_owned(),
                                <SecretToken as CredentialState>::VERSION,
                                None,
                            ),
                        ),
                    ),
                ),
            )
            .await
            .expect("reauthentication completed");
    }

    /// Acquires a refresh claim and marks its provider boundary crossed: new
    /// uses see a refresh in flight.
    async fn claim_refresh_crossing(&self) -> nebula_storage_port::store::ClaimToken {
        let claimed = self
            .claims
            .try_claim(
                &self.selector(),
                &ReplicaId::new("refreshing-replica"),
                Duration::from_secs(30),
                CredentialOperationIntent::Refresh,
            )
            .await
            .expect("the refresh claim reaches the backend");
        let ClaimAttempt::Acquired(claim) = claimed else {
            panic!("a fresh credential's refresh claim is acquired");
        };
        self.claims
            .mark_sentinel(&claim.token)
            .await
            .expect("the refresh crosses the provider boundary");
        claim.token
    }
}

fn unavailable_reason(error: &ResourceError) -> Option<CredentialUnavailableReason> {
    match error.kind() {
        nebula_resource::ErrorKind::CredentialUnavailable { reason } => Some(*reason),
        _ => None,
    }
}

fn refusal<T>(result: Result<T, ResourceError>) -> ResourceError {
    match result {
        Ok(_) => panic!("expected a refusal"),
        Err(error) => error,
    }
}

/// A: the claim is acquired and lapses with no activation in between. The
/// next activation sees only the next use revision at the same material and
/// readmits the kept registration: no registration, no projection, the
/// admitted lease left open.
#[tokio::test]
async fn an_abandoned_revoke_between_activations_readmits_the_row() {
    let fixture = SqliteFixture::new().await;
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    assert_eq!(drain(&mut events).0, 1);
    let (material, admission) = fixture.tracked_at(resource_id);
    let lease = fixture.acquire(&key, &activated).await.expect("serves");
    let projections = fixture.projections();

    fixture.claim_revoke().await;
    fixture.lapse_claim().await;

    assert_eq!(
        fixture
            .activate(resource_id, &key)
            .await
            .expect("activates"),
        activated
    );
    assert_eq!(drain(&mut events), (0, 0), "nothing registers again");
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");
    assert!(!lease.is_closing(), "a readmission closes no lease");
    assert_eq!(fixture.tracked_at(resource_id), (material, admission + 1));
    assert_eq!(
        fixture.gate_outcome(
            &activated,
            CredentialObservedAt::new(material).with_admission_epoch(admission + 1)
        ),
        nebula_resource::CredentialReopenOutcome::NotSuspended,
        "the activation already admitted the next use revision"
    );
    let fresh = fixture.acquire(&key, &activated).await.expect("serves");
    assert!(!fresh.is_closing());
}

/// B: an activation observes the claim in flight and suspends the row; once
/// the claim lapses, the next activation reopens it at the next use
/// revision. The lease admitted before the claim stays closed.
#[tokio::test]
async fn an_observed_revoke_suspends_and_its_lapse_reopens_at_the_next_revision() {
    let fixture = SqliteFixture::new().await;
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    drain(&mut events);
    let (material, admission) = fixture.tracked_at(resource_id);
    let old = fixture.acquire(&key, &activated).await.expect("serves");

    fixture.claim_revoke().await;
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::Credential {
            source: CredentialSlotResolveError::OperationBlocked { .. },
            ..
        })
    );
    assert!(fixture.suspended(&activated));
    assert!(
        old.is_closing(),
        "the lease admitted before the block closes"
    );

    fixture.lapse_claim().await;
    assert_eq!(
        fixture
            .activate(resource_id, &key)
            .await
            .expect("activates"),
        activated
    );
    assert!(!fixture.suspended(&activated));
    assert_eq!(fixture.tracked_at(resource_id), (material, admission + 1));
    assert_eq!(drain(&mut events), (0, 0), "nothing registers again");
    assert!(old.is_closing(), "the old lease stays closed");
    let fresh = fixture.acquire(&key, &activated).await.expect("serves");
    assert!(!fresh.is_closing());
}

/// C: a display rename moves the aggregate revision only; the row is not
/// registered again and nothing is decrypted.
#[tokio::test]
async fn a_display_rename_does_not_register_the_row_again() {
    let fixture = SqliteFixture::new().await;
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    drain(&mut events);
    let at = fixture.tracked_at(resource_id);
    let projections = fixture.projections();

    let head = fixture
        .store
        .get_head(&fixture.selector())
        .await
        .expect("head");
    fixture
        .store
        .replace(
            &fixture.selector(),
            CredentialReplacement::new(
                head.version(),
                Some("renamed".to_owned()),
                false,
                display("renamed"),
                CredentialMaterialTransition::preserve(RefreshRetryTransition::Preserve),
            ),
        )
        .await
        .expect("rename");
    assert!(
        fixture
            .store
            .get_head(&fixture.selector())
            .await
            .expect("head")
            .version()
            > head.version(),
        "the rename moved the aggregate revision"
    );

    assert_eq!(
        fixture
            .activate(resource_id, &key)
            .await
            .expect("activates"),
        activated
    );
    assert_eq!(drain(&mut events), (0, 0), "a rename registers nothing");
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");
    assert_eq!(fixture.tracked_at(resource_id), at);
}

// ── Strict per-acquire admission over the real runtime ─────────────────────

/// S1: a revoke claim held is seen by the acquire itself — no activation in
/// between — and suspends the row; its lapse reopens the row by the next
/// acquire alone, at the next use revision.
#[tokio::test]
async fn a_strict_acquire_refuses_a_held_revoke_and_reopens_by_itself_after_it_lapses() {
    let fixture = SqliteFixture::strict().await;
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    drain(&mut events);
    let (material, admission) = fixture.tracked_at(resource_id);
    let old = fixture.acquire(&key, &activated).await.expect("serves");
    let projections = fixture.projections();

    fixture.claim_revoke().await;
    let error = refusal(fixture.acquire(&key, &activated).await);
    assert_eq!(
        unavailable_reason(&error),
        Some(CredentialUnavailableReason::OperationBlocked)
    );
    assert!(fixture.suspended(&activated));
    assert!(old.is_closing(), "the lease admitted before closes");
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");

    fixture.lapse_claim().await;
    let fresh = fixture
        .acquire(&key, &activated)
        .await
        .expect("the acquire alone reopens the row");
    assert!(!fresh.is_closing());
    assert!(!fixture.suspended(&activated));
    assert!(old.is_closing(), "the old lease stays closed");
    assert_eq!(fixture.projections(), projections);
    assert_eq!(
        fixture.gate_outcome(
            &activated,
            CredentialObservedAt::new(material).with_admission_epoch(admission + 1)
        ),
        nebula_resource::CredentialReopenOutcome::NotSuspended,
        "the acquire reopened at the next use revision"
    );
    assert_eq!(drain(&mut events), (0, 0), "nothing registers or retires");
}

/// S2: reauthentication required refuses and suspends; its completion with
/// new material refuses as rebinding until activation installs it.
#[tokio::test]
async fn a_strict_acquire_waits_for_reauthentication_and_then_for_its_material() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    drop(fixture.acquire(&key, &activated).await.expect("serves"));

    fixture.require_reauth().await;
    let error = refusal(fixture.acquire(&key, &activated).await);
    assert_eq!(
        unavailable_reason(&error),
        Some(CredentialUnavailableReason::ReauthRequired)
    );
    assert!(fixture.suspended(&activated));

    fixture.complete_reauth().await;
    let error = refusal(fixture.acquire(&key, &activated).await);
    assert_eq!(
        unavailable_reason(&error),
        Some(CredentialUnavailableReason::Rebinding),
        "new material is not installed yet"
    );

    let reactivated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activation installs the new material");
    let lease = fixture
        .acquire(&key, &reactivated)
        .await
        .expect("serves on the new material");
    assert!(!lease.is_closing());
}

/// S3: a credential store outage refuses new credentialed work without
/// suspending or retiring anything; the restored store serves again without
/// a projection.
#[tokio::test]
async fn a_strict_acquire_refuses_during_a_store_outage_and_serves_after_it() {
    let fixture = SqliteFixture::strict().await;
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    drain(&mut events);
    let projections = fixture.projections();

    fixture.outage.store(true, Ordering::SeqCst);
    for _ in 0..3 {
        let error = refusal(fixture.acquire(&key, &activated).await);
        assert_eq!(
            unavailable_reason(&error),
            Some(CredentialUnavailableReason::CheckUnavailable)
        );
    }
    assert!(!fixture.suspended(&activated), "an outage suspends nothing");

    fixture.outage.store(false, Ordering::SeqCst);
    drop(fixture.acquire(&key, &activated).await.expect("serves"));
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");
    assert_eq!(drain(&mut events), (0, 0), "nothing registers or retires");
}

/// S4: a refresh crossing the provider boundary is joined; released without
/// new material while the acquire waits, it admits.
#[tokio::test]
async fn a_strict_acquire_joins_a_refresh_in_flight() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let token = fixture.claim_refresh_crossing().await;
    let reads = fixture.reads();

    let release = async {
        // The acquire has read the refresh in flight and is waiting on it.
        while fixture.reads() == reads {
            tokio::task::yield_now().await;
        }
        fixture
            .claims
            .release(token)
            .await
            .expect("the refresh releases without new material");
    };
    let (lease, ()) = tokio::join!(fixture.acquire(&key, &activated), release);
    let lease = lease.expect("admitted once the refresh released");
    assert!(!lease.is_closing());
    assert!(fixture.reads() >= reads + 2, "joined, then read again");
    assert!(!fixture.suspended(&activated));
}

/// S5: a resource without credential slots reads nothing on a strict
/// manager.
#[tokio::test]
async fn a_slot_less_resource_on_a_strict_manager_reads_nothing() {
    let fixture = SqliteFixture::strict().await;
    fixture
        .manager
        .register(nebula_resource::RegistrationSpec {
            resource: Plain,
            config: LabelConfig {
                label: "a".to_owned(),
            },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::from_bindings(std::iter::empty::<(&str, &str)>()),
            topology: Resident::new(resident::config::Config::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    let ctx = ResourceContext::minimal(
        nebula_core::scope::Scope::default(),
        CancellationToken::new(),
    );
    drop(
        fixture
            .manager
            .acquire::<Plain>(&ctx, &nebula_resource::AcquireOptions::default())
            .await
            .expect("serves"),
    );
    assert_eq!(fixture.reads(), 0);
}

// ── Strict per-attempt admission through the managed call facade ───────────

/// `n` free read attempts, each settled `Sent`, on any row.
struct Attempts(u32);

impl<R: Provider + PinSlots> Operation<R> for Attempts {
    type Output = ();
    const EFFECT: Effect = Effect::Read;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(self.0).expect("at least one attempt")
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<(), OperationError> {
        for _ in 0..self.0 {
            let attempt = cx.attempt(Cost::ONE).await?;
            attempt.settle(SentState::Sent);
        }
        Ok(())
    }
}

/// Two read attempts on the bearer row; after the first is sent it signals
/// `between` and waits for `resume`. Yields the material each attempt was
/// pinned on.
struct PausedRead {
    between: Arc<Notify>,
    resume: Arc<Notify>,
}

fn paused_read() -> (PausedRead, Arc<Notify>, Arc<Notify>) {
    let (between, resume) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    (
        PausedRead {
            between: Arc::clone(&between),
            resume: Arc::clone(&resume),
        },
        between,
        resume,
    )
}

impl Operation<BearerRow> for PausedRead {
    type Output = Vec<Option<u64>>;
    const EFFECT: Effect = Effect::Read;

    fn max_attempts(&self) -> NonZeroU32 {
        NonZeroU32::new(2).expect("two")
    }

    async fn run(
        self,
        cx: &mut OperationCx<'_, BearerRow>,
    ) -> Result<Self::Output, OperationError> {
        let first = cx.attempt(Cost::ONE).await?;
        let mut pinned = vec![first.credentials().as_ref().map(|(material, _)| *material)];
        first.settle(SentState::Sent);
        self.between.notify_one();
        self.resume.notified().await;
        let second = cx.attempt(Cost::ONE).await?;
        pinned.push(second.credentials().as_ref().map(|(material, _)| *material));
        second.settle(SentState::Sent);
        Ok(pinned)
    }
}

fn op_reason(error: &OperationError) -> Option<CredentialUnavailableReason> {
    match error.kind() {
        nebula_resource::ErrorKind::CredentialUnavailable { reason } => Some(*reason),
        _ => None,
    }
}

/// F1: every attempt of a unit reads the credential once; nothing is
/// decrypted.
#[tokio::test]
async fn every_managed_attempt_reads_the_credential_once() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let managed = fixture.facade(&key, &activated).await;
    let (reads, projections) = (fixture.reads(), fixture.projections());

    managed.submit(Attempts(3)).await.expect("granted");
    assert_eq!(fixture.reads(), reads + 3);
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");
}

/// F2: a revoke claim taken between two attempts refuses the second before
/// it is sent, suspends the row and closes the lease; the next unit on the
/// closed lease is refused without a read; the claim's lapse lets a new
/// acquire serve again.
#[tokio::test]
async fn a_revoke_between_attempts_refuses_the_next_attempt_and_closes_the_lease() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let managed = fixture.facade(&key, &activated).await;

    let (operation, between, resume) = paused_read();
    let unit = tokio::spawn(managed.submit(operation));
    between.notified().await;
    fixture.claim_revoke().await;
    resume.notify_one();

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(
        op_reason(&error),
        Some(CredentialUnavailableReason::OperationBlocked)
    );
    assert_eq!(
        error.sent(),
        SentState::Sent,
        "only the first attempt was sent"
    );
    assert!(error.is_retryable(), "a read unit is retried");
    assert!(fixture.suspended(&activated));
    assert!(managed.is_closing(), "the suspension closes the lease");

    let reads = fixture.reads();
    let error = managed.submit(Attempts(1)).await.expect_err("closed lease");
    assert_eq!(
        op_reason(&error),
        Some(CredentialUnavailableReason::OperationBlocked)
    );
    assert_eq!(error.sent(), SentState::NotSent);
    assert_eq!(fixture.reads(), reads, "refused before any read");

    fixture.lapse_claim().await;
    fixture
        .facade(&key, &activated)
        .await
        .submit(Attempts(1))
        .await
        .expect("a new acquire serves once the claim lapsed");
    assert!(!fixture.suspended(&activated));
}

/// F3: a store outage between two attempts refuses the second as
/// unchecked, without suspending the row or closing the lease; once the
/// store is back the same facade serves, with nothing decrypted.
#[tokio::test]
async fn an_outage_between_attempts_refuses_without_closing_the_lease() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let managed = fixture.facade(&key, &activated).await;
    let projections = fixture.projections();

    let (operation, between, resume) = paused_read();
    let unit = tokio::spawn(managed.submit(operation));
    between.notified().await;
    fixture.outage.store(true, Ordering::SeqCst);
    resume.notify_one();

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(
        op_reason(&error),
        Some(CredentialUnavailableReason::CheckUnavailable)
    );
    assert!(!fixture.suspended(&activated), "an outage suspends nothing");
    assert!(!managed.is_closing(), "an outage closes no lease");

    fixture.outage.store(false, Ordering::SeqCst);
    managed
        .submit(Attempts(1))
        .await
        .expect("the same facade serves after the outage");
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");
}

/// F4: reauthentication completed with new material between two attempts
/// refuses the second as rebinding; activation installs the new material
/// and a new acquire serves on it.
#[tokio::test]
async fn new_material_between_attempts_refuses_the_next_attempt_as_rebinding() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let managed = fixture.facade(&key, &activated).await;

    let (operation, between, resume) = paused_read();
    let unit = tokio::spawn(managed.submit(operation));
    between.notified().await;
    fixture.complete_reauth().await;
    resume.notify_one();

    let error = unit.await.expect("joined").expect_err("refused");
    assert_eq!(
        op_reason(&error),
        Some(CredentialUnavailableReason::Rebinding)
    );
    assert!(!fixture.suspended(&activated));

    let reactivated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activation installs the new material");
    let (operation, between, resume) = paused_read();
    let unit = tokio::spawn(fixture.facade(&key, &reactivated).await.submit(operation));
    between.notified().await;
    resume.notify_one();
    let pinned = unit.await.expect("joined").expect("serves");
    assert!(
        pinned[0].is_some() && pinned[0] == pinned[1],
        "both attempts run on the installed material"
    );
}

/// F5: a refresh crossing the provider boundary between two attempts is
/// joined by the second attempt's read; released without new material, the
/// attempt is admitted.
#[tokio::test]
async fn a_refresh_between_attempts_is_joined_and_then_admitted() {
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let managed = fixture.facade(&key, &activated).await;

    let (operation, between, resume) = paused_read();
    let unit = tokio::spawn(managed.submit(operation));
    between.notified().await;
    let token = fixture.claim_refresh_crossing().await;
    let reads = fixture.reads();
    resume.notify_one();
    // The second attempt has read the refresh in flight and is joining it.
    while fixture.reads() == reads {
        tokio::task::yield_now().await;
    }
    fixture
        .claims
        .release(token)
        .await
        .expect("the refresh releases without new material");

    let pinned = unit.await.expect("joined").expect("admitted");
    assert_eq!(pinned[0], pinned[1], "the unit's pin is unchanged");
    assert!(fixture.reads() >= reads + 2, "joined, then read again");
    assert!(!fixture.suspended(&activated));
    assert!(!managed.is_closing());
}

/// F6: a slot-less resource's facade reads nothing on a strict manager.
#[tokio::test]
async fn a_slot_less_facade_on_a_strict_manager_reads_nothing() {
    let fixture = SqliteFixture::strict().await;
    fixture
        .manager
        .register(nebula_resource::RegistrationSpec {
            resource: Plain,
            config: LabelConfig {
                label: "a".to_owned(),
            },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::from_bindings(std::iter::empty::<(&str, &str)>()),
            topology: Resident::new(resident::config::Config::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    let ctx = ResourceContext::minimal(
        nebula_core::scope::Scope::default(),
        CancellationToken::new(),
    );
    let managed = fixture
        .manager
        .acquire::<Plain>(&ctx, &nebula_resource::AcquireOptions::default())
        .await
        .expect("serves")
        .into_lease();
    managed.submit(Attempts(2)).await.expect("granted");
    assert_eq!(fixture.reads(), 0);
}

// ── F7: the action path to a managed row ───────────────────────────────────

/// An action context whose resources are the engine's accessor over
/// `manager`, at `scope`'s workspace, serving `key` under `identity`, and
/// cancelled with `cancel`.
fn action_context(
    manager: &Arc<Manager>,
    scope: &Scope,
    key: &ResourceKey,
    identity: &SlotIdentity,
    cancel: &CancellationToken,
) -> nebula_action::ActionRuntimeContext {
    let workspace = WorkspaceId::parse(&scope.workspace_id).expect("workspace id");
    let accessor = crate::resource_accessor::EngineResourceAccessor::new(
        Arc::clone(manager),
        nebula_core::scope::Scope {
            workspace_id: Some(workspace),
            ..Default::default()
        },
        cancel.clone(),
    )
    .with_slot_identities(std::collections::HashMap::from([(
        key.clone(),
        identity.clone(),
    )]));
    nebula_action::testing::TestContextBuilder::new()
        .build()
        .with_resources(Arc::new(accessor))
}

/// The bearer row as an action resolves it.
fn action_row(
    fixture: &SqliteFixture,
    key: &ResourceKey,
    identity: &SlotIdentity,
) -> Result<nebula_resource::call::ResourceHandle<BearerRow>, nebula_action::ActionError> {
    use nebula_action::ActionContextExt as _;
    action_context(
        &fixture.manager,
        &fixture.scope,
        key,
        identity,
        &CancellationToken::new(),
    )
    .resource_handle_by_id::<BearerRow>(key.as_str())
}

/// F7: an action's managed row reads the credential once per attempt
/// through the real runtime; a revoke claim or a reauthentication flag
/// refuses its unit unsent, retryable with the reason's hint; another
/// identity's row is not served.
#[tokio::test]
async fn an_action_resource_handle_reads_per_attempt_and_refuses_retryably() {
    // One read per attempt, nothing decrypted.
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let row = action_row(&fixture, &key, &activated.slot_identity).expect("the action's row");
    row.submit(Attempts(1)).await.expect("warm");
    let (reads, projections) = (fixture.reads(), fixture.projections());
    row.submit(Attempts(1)).await.expect("granted");
    // A resident checkout clones the master, which counts as a create:
    // each attempt reads before and after its checkout (R1, R2).
    let per_attempt = fixture.reads() - reads;
    assert!(per_attempt >= 1, "every attempt reads");
    let reads = fixture.reads();
    row.submit(Attempts(3)).await.expect("granted");
    assert_eq!(
        fixture.reads(),
        reads + 3 * per_attempt,
        "reads per attempt"
    );
    assert_eq!(fixture.projections(), projections, "nothing is decrypted");

    // A held revoke claim refuses the next unit, unsent and retryable.
    fixture.claim_revoke().await;
    let refused = row.submit(Attempts(1)).await.expect_err("revoke claim");
    assert_eq!(
        op_reason(&refused),
        Some(CredentialUnavailableReason::OperationBlocked)
    );
    assert_eq!(refused.sent(), SentState::NotSent);
    let error = nebula_action::ActionError::from(refused);
    assert!(
        matches!(error, nebula_action::ActionError::Retryable { .. }),
        "{error}"
    );
    assert_eq!(
        error.backoff_hint(),
        Some(CredentialUnavailableReason::OperationBlocked.retry_after())
    );

    // A reauthentication flag refuses likewise, with its own hint.
    let fixture = SqliteFixture::strict().await;
    let (resource_id, key) = fixture.store_row().await;
    let activated = fixture
        .activate(resource_id, &key)
        .await
        .expect("activates");
    let row = action_row(&fixture, &key, &activated.slot_identity).expect("the action's row");
    fixture.require_reauth().await;
    let refused = row.submit(Attempts(1)).await.expect_err("reauth");
    assert_eq!(
        op_reason(&refused),
        Some(CredentialUnavailableReason::ReauthRequired)
    );
    assert_eq!(refused.sent(), SentState::NotSent);
    let error = nebula_action::ActionError::from(refused);
    assert!(
        matches!(error, nebula_action::ActionError::Retryable { .. }),
        "{error}"
    );
    assert_eq!(
        error.backoff_hint(),
        Some(CredentialUnavailableReason::ReauthRequired.retry_after())
    );

    // Another identity's row is not served: fatal at resolution.
    let stranger = CredentialId::new().to_string();
    let other = SlotIdentity::from_bindings([(AUTH_SLOT, stranger.as_str())]);
    let error = action_row(&fixture, &key, &other).expect_err("not this identity's row");
    assert!(
        matches!(error, nebula_action::ActionError::Fatal { .. }),
        "{error}"
    );
}

#[path = "session_postgres_tests.rs"]
mod session_postgres;
