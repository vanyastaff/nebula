//! Storage-backed acceptance of the credential use revision.
//!
//! The real read-only credential runtime (`CredentialProjectionRuntime`)
//! over encrypted SQLite persistence and the real SQLite refresh-claim
//! repository: the admission epoch activation compares is the one the
//! backend writes, not a scripted value. A revoke claim acquired and then
//! abandoned closes use by itself (the claim bumps the epoch) and reopens by
//! expiry alone, with no write in between.
use std::{str::FromStr, time::Duration};

use nebula_credential::{
    BearerTokenCredential, Credential, CredentialProjectionRuntime, CredentialRegistry,
    CredentialState, DispatchOps, ErasedPendingStore, SecretString, StateSource,
    register_runtime_ops, scheme::SecretToken,
};
use nebula_storage::credential::{
    EncryptionLayer, EnvKeyProvider, SqliteCredentialPersistence, SqliteRefreshClaimRepo,
};
use nebula_storage_port::{
    CredentialCreate, CredentialMaterialEpoch, CredentialMaterialTransition, CredentialOwner,
    CredentialPersistence, CredentialReplacement, CredentialSelector, RefreshRetryTransition,
    SecretBytes,
    store::{ClaimAttempt, CredentialOperationIntent, RefreshClaimStore, ReplicaId},
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

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
    inner: CredentialProjectionRuntime,
    projections: AtomicUsize,
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
    _directory: tempfile::TempDir,
}

impl SqliteFixture {
    async fn new() -> Self {
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
        let store: Arc<dyn CredentialPersistence> = Arc::new(EncryptionLayer::new(raw, key));

        let mut registry = CredentialRegistry::new();
        registry
            .register(BearerTokenCredential, "nebula-engine-test")
            .expect("bearer registration");
        let mut ops = DispatchOps::<ErasedPendingStore>::new();
        register_runtime_ops::<BearerTokenCredential, ErasedPendingStore>(&mut ops)
            .expect("runtime ops");
        let runtime = CredentialProjectionRuntime::from_secure_parts(
            Arc::clone(&store),
            Arc::new(registry),
            Arc::new(ops),
            StateSource::LocalEncrypted,
        )
        .expect("projection runtime");

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
            manager: Arc::new(Manager::new()),
            expr_engine: ExpressionEngine::with_cache_size(16),
            scope,
            cancel: CancellationToken::new(),
            credential_id,
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
        let resource_id = ResourceId::new();
        self.resources
            .create(
                &self.scope,
                ResourceRow {
                    id: resource_id.to_string(),
                    workspace_id: self.scope.workspace_id.clone(),
                    slug: format!("row-{resource_id}"),
                    display_name: "row".to_owned(),
                    kind: BEARER_KIND.to_owned(),
                    config: serde_json::json!({ "label": "a" }),
                    credential_bindings: BTreeMap::from([(
                        AUTH_SLOT.to_owned(),
                        self.credential_id.to_string(),
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
            ResourceKey::new(BEARER_KIND).expect("valid resource key"),
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
