use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use nebula_core::{
    CredentialKey, DeclaresDependencies, Dependencies, ResourceKey,
    dependencies::{SlotField, SlotKind},
    resource_key,
};
use nebula_resource::{
    KindActivator, Resident, ResourceContext, ResourceEvent,
    error::Error as ResourceError,
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::resident,
};
use nebula_storage::inmem::InMemoryResourceStore;

use super::*;

// ── Test resources ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize, nebula_schema::Schema)]
struct LabelConfig {
    #[serde(default)]
    #[field(default = "")]
    label: String,
}

impl ResourceConfig for LabelConfig {
    fn validate(&self) -> Result<(), ResourceError> {
        if self.label.is_empty() {
            return Err(ResourceError::permanent("label must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Clone)]
struct Plain;

#[async_trait::async_trait]
impl Provider for Plain {
    type Config = LabelConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("activation.plain")
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
            nebula_resource::metadata_name!("activation.plain"),
            String::new(),
        )
    }
}

nebula_resource::no_credential_slots!(Plain);

impl DeclaresDependencies for Plain {
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

impl resident::ResidentProvider for Plain {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

/// Names an account credential slot (`billing`) it does not declare.
#[derive(Clone)]
struct Misnamed;

#[async_trait::async_trait]
impl Provider for Misnamed {
    type Config = LabelConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("activation.misnamed")
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
            nebula_resource::metadata_name!("activation.misnamed"),
            String::new(),
        )
    }

    fn resilience() -> nebula_resource::rate_limit::ResiliencePolicy {
        nebula_resource::rate_limit::ResiliencePolicy::new().account_credential("billing")
    }
}

nebula_resource::no_credential_slots!(Misnamed);

impl DeclaresDependencies for Misnamed {
    fn dependencies() -> Dependencies {
        Dependencies::new()
    }
}

impl resident::ResidentProvider for Misnamed {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

const AUTH_SLOT: &str = "auth";

/// One credential slot, with the complete projection contract rotation
/// needs (a derived resource gets the same from `#[derive(Resource)]`).
#[derive(Clone)]
struct Slotted {
    auth: Arc<nebula_resource::SlotCell<nebula_credential::CredentialGuard<String>>>,
}

impl Slotted {
    fn new() -> Self {
        Self {
            auth: Arc::new(nebula_resource::SlotCell::empty()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Slotted {
    type Config = LabelConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("activation.slotted")
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
            nebula_resource::metadata_name!("activation.slotted"),
            String::new(),
        )
    }
}

impl DeclaresDependencies for Slotted {
    fn dependencies() -> Dependencies {
        Dependencies::new().slot_field(SlotField {
            slot_key: AUTH_SLOT,
            default_id: AUTH_SLOT,
            kind: SlotKind::Credential {
                type_id: std::any::TypeId::of::<()>(),
                type_name: "test-credential",
                key: CredentialKey::new("auth").expect("valid credential key"),
            },
            required: true,
            lazy: false,
            purpose: None,
        })
    }
}

impl nebula_resource::HasCredentialSlots for Slotted {
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
        let (metadata, guard) = typed(slot, guard)?;
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
        let (metadata, guard) = typed(slot, guard)?;
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

/// The `auth` slot's guard, typed, with the metadata it was projected with.
fn typed(
    slot: &str,
    guard: nebula_credential::ErasedCredentialGuard,
) -> Result<
    (
        nebula_credential::CredentialGuardMetadata,
        Arc<nebula_credential::CredentialGuard<String>>,
    ),
    nebula_resource::SlotInstallError,
> {
    if slot != AUTH_SLOT {
        return Err(nebula_resource::SlotInstallError::UnknownSlot);
    }
    let metadata = guard.metadata().clone();
    let guard = guard
        .into_typed::<String>()
        .map_err(|_| nebula_resource::SlotInstallError::CredentialTypeMismatch)?;
    Ok((metadata, Arc::new(guard)))
}

impl resident::ResidentProvider for Slotted {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

/// Answers every credential with the scripted outcome (by default a
/// refusal), a `(material_epoch, revision)` for a guard, and counts the
/// attempts.
struct ScriptedResolver {
    calls: AtomicUsize,
    outcome: std::sync::Mutex<Result<(u64, u64), CredentialSlotResolveError>>,
    /// Never answer, as a stalled credential backend.
    stall: std::sync::atomic::AtomicBool,
    /// Answer the calls before this one, then stall.
    stall_from: AtomicUsize,
}

impl Default for ScriptedResolver {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            outcome: std::sync::Mutex::new(Err(CredentialSlotResolveError::NotFound)),
            stall: std::sync::atomic::AtomicBool::new(false),
            stall_from: AtomicUsize::new(usize::MAX),
        }
    }
}

impl ScriptedResolver {
    fn answer(&self, outcome: Result<(u64, u64), CredentialSlotResolveError>) {
        *self.outcome.lock().unwrap() = outcome;
    }
}

impl CredentialSlotResolver for ScriptedResolver {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        _required_capabilities: Capabilities,
        _cancel: CancellationToken,
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
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let outcome = (*self.outcome.lock().unwrap()).map(|(material_epoch, revision)| {
            nebula_credential::ErasedCredentialGuard::from_typed(
                nebula_credential::CredentialGuard::new(String::from("secret")),
                // Owner-qualified, as a production resolver stamps it.
                nebula_credential::CredentialGuardMetadata::new(
                    credential_id,
                    expected_key,
                    material_epoch,
                    revision,
                )
                .with_scope(scope.durable_owner_scope()),
            )
        });
        let stall =
            self.stall.load(Ordering::SeqCst) || call >= self.stall_from.load(Ordering::SeqCst);
        Box::pin(async move {
            if stall {
                std::future::pending::<()>().await;
            }
            outcome
        })
    }
}

// ── Fixture ────────────────────────────────────────────────────────────────

pub(crate) fn registrars() -> ResourceActivatorRegistry {
    let mut registrars = ResourceActivatorRegistry::new();
    registrars
        .insert(
            "activation.plain",
            Arc::new(KindActivator::<Plain, _, _>::new(
                || Plain,
                nebula_resource::topology::fixed(|| {
                    Resident::<Plain>::new(resident::config::Config::default())
                }),
            )),
        )
        .expect("plain resource admits");
    registrars
        .insert(
            "activation.slotted",
            Arc::new(KindActivator::<Slotted, _, _>::new(
                Slotted::new,
                nebula_resource::topology::fixed(|| {
                    Resident::<Slotted>::new(resident::config::Config::default())
                }),
            )),
        )
        .expect("slotted resource admits");
    registrars
        .insert(
            "activation.misnamed",
            Arc::new(KindActivator::<Misnamed, _, _>::new(
                || Misnamed,
                nebula_resource::topology::fixed(|| {
                    Resident::<Misnamed>::new(resident::config::Config::default())
                }),
            )),
        )
        .expect("misnamed resource admits");
    registrars
}

struct Fixture {
    store: Arc<InMemoryResourceStore>,
    activator: StoredResourceActivator,
    registrars: ResourceActivatorRegistry,
    manager: Arc<Manager>,
    expr_engine: ExpressionEngine,
    resolver: Arc<ScriptedResolver>,
    scope: Scope,
    cancel: CancellationToken,
    /// Attached to `manager`, as the engine attaches its index.
    #[cfg(feature = "rotation")]
    fanout: Arc<nebula_resource::ResourceFanoutIndex>,
    /// Activate as the engine does while a fan-out driver reconciles
    /// `fanout`; the test spawns that driver itself.
    #[cfg(feature = "rotation")]
    live_fanout: bool,
    /// The driver's credential bus: it runs while the bus lives.
    #[cfg(feature = "rotation")]
    credential_events: Arc<nebula_eventbus::EventBus<nebula_credential::CredentialEvent>>,
}

impl Fixture {
    fn new() -> Self {
        let store = Arc::new(InMemoryResourceStore::new());
        let manager = Arc::new(Manager::new());
        #[cfg(feature = "rotation")]
        let fanout = Arc::new(nebula_resource::ResourceFanoutIndex::new());
        #[cfg(feature = "rotation")]
        manager.attach_rotation_index(&fanout);
        Self {
            activator: StoredResourceActivator::new(Arc::clone(&store) as Arc<dyn ResourceStore>),
            store,
            registrars: registrars(),
            manager,
            expr_engine: ExpressionEngine::with_cache_size(16),
            resolver: Arc::new(ScriptedResolver::default()),
            scope: Scope::new(
                WorkspaceId::new().to_string(),
                nebula_core::OrgId::new().to_string(),
            ),
            cancel: CancellationToken::new(),
            #[cfg(feature = "rotation")]
            fanout,
            #[cfg(feature = "rotation")]
            live_fanout: false,
            #[cfg(feature = "rotation")]
            credential_events: Arc::new(nebula_eventbus::EventBus::new(8)),
        }
    }

    /// Spawns a fan-out driver rereading credentials through the fixture's
    /// resolver and activates as the engine does while one runs. Dropping
    /// the handle stops the driver.
    #[cfg(feature = "rotation")]
    fn start_fanout(&mut self) -> nebula_resource::ResourceFanoutDriver {
        self.live_fanout = true;
        nebula_resource::ResourceFanoutDriver::spawn_with_resolver(
            Arc::clone(&self.fanout),
            Arc::clone(&self.manager),
            Some(Arc::clone(&self.resolver) as Arc<dyn CredentialSlotResolver>),
            Arc::clone(&self.credential_events),
            None,
        )
    }

    fn context(&self, with_credentials: bool) -> ActivationContext<'_> {
        ActivationContext {
            registrars: &self.registrars,
            manager: &self.manager,
            credentials: with_credentials
                .then_some(self.resolver.as_ref() as &dyn CredentialSlotResolver),
            expr_engine: &self.expr_engine,
            #[cfg(feature = "rotation")]
            fanout: self.live_fanout.then_some(&self.fanout),
        }
    }

    async fn store_row(
        &self,
        kind: &str,
        label: &str,
        bindings: &[(&str, &str)],
    ) -> (ResourceId, ResourceKey) {
        self.store_row_with_settings(kind, label, bindings, None, None)
            .await
    }

    async fn store_row_with_settings(
        &self,
        kind: &str,
        label: &str,
        bindings: &[(&str, &str)],
        topology: Option<serde_json::Value>,
        resilience_override: Option<serde_json::Value>,
    ) -> (ResourceId, ResourceKey) {
        let resource_id = ResourceId::new();
        self.store
            .create(
                &self.scope,
                ResourceRow {
                    id: resource_id.to_string(),
                    workspace_id: self.scope.workspace_id.clone(),
                    slug: format!("row-{resource_id}"),
                    display_name: "row".to_owned(),
                    kind: kind.to_owned(),
                    config: serde_json::json!({ "label": label }),
                    credential_bindings: bindings
                        .iter()
                        .map(|(slot, selector)| ((*slot).to_owned(), (*selector).to_owned()))
                        .collect::<BTreeMap<_, _>>(),
                    topology,
                    resilience_override,
                    created_at: "2026-09-23T00:00:00Z".to_owned(),
                    created_by: "test".to_owned(),
                    version: 0,
                    deleted_at: None,
                },
            )
            .await
            .expect("row stored");
        let key = ResourceKey::new(kind).expect("valid resource key");
        (resource_id, key)
    }

    async fn activate(
        &self,
        resource_id: ResourceId,
        key: &ResourceKey,
    ) -> Result<ActivatedResource, StoredResourceActivationError> {
        self.activator
            .activate(
                &self.context(true),
                &self.scope,
                resource_id,
                key,
                &self.cancel,
            )
            .await
    }
}

fn drain(events: &mut nebula_resource::Subscriber<ResourceEvent>) -> (usize, usize) {
    let (mut registered, mut removed) = (0, 0);
    while let Some(event) = events.try_recv() {
        match event {
            ResourceEvent::Registered { .. } => registered += 1,
            ResourceEvent::Removed { .. } => removed += 1,
            _ => {},
        }
    }
    (registered, removed)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_row_activates_once_per_version_even_under_concurrency() {
    let fixture = Fixture::new();
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row("activation.plain", "a", &[]).await;

    let outcomes =
        futures::future::join_all((0..32).map(|_| fixture.activate(resource_id, &key))).await;
    let first = outcomes[0].as_ref().expect("row activates").clone();
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.as_ref().ok() == Some(&first))
    );
    assert_eq!(
        first.slot_identity,
        SlotIdentity::from_row_bindings(Some(&resource_id.to_string()), std::iter::empty())
    );
    assert_eq!(
        first.scope,
        ScopeLevel::Workspace(WorkspaceId::parse(&fixture.scope.workspace_id).unwrap())
    );
    assert_eq!(
        drain(&mut events),
        (1, 0),
        "32 concurrent turns, one registration"
    );

    // A new stored version re-registers in place under the same identity.
    let mut row = fixture
        .store
        .get(&fixture.scope, &resource_id.to_string())
        .await
        .unwrap()
        .unwrap();
    row.config = serde_json::json!({ "label": "b" });
    row.version = 1;
    fixture.store.update(&fixture.scope, row, 0).await.unwrap();
    assert_eq!(fixture.activate(resource_id, &key).await.unwrap(), first);
    assert_eq!(
        drain(&mut events),
        (1, 0),
        "a version bump re-registers once"
    );
}

/// A retired row leaves the credential-rotation index too — the manager
/// prunes the index attached to it — so a later refresh of its credentials
/// is not dispatched to a row that is gone.
#[cfg(feature = "rotation")]
#[tokio::test]
async fn a_retired_row_leaves_the_rotation_index() {
    let fixture = Fixture::new();
    let (resource_id, key) = fixture.store_row("activation.plain", "a", &[]).await;
    let activated = fixture.activate(resource_id, &key).await.unwrap();
    let credential = CredentialId::new();
    fixture.fanout.bind(
        credential,
        activated.resource_key.clone(),
        activated.scope.clone(),
        "token",
        activated.slot_identity.clone(),
    );
    assert_eq!(fixture.fanout.affected(&credential).len(), 1);

    fixture
        .store
        .soft_delete(&fixture.scope, &resource_id.to_string())
        .await
        .unwrap();
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::NotFound { .. })
    );
    assert!(
        fixture.fanout.affected(&credential).is_empty(),
        "retiring the row unbinds it from rotation fan-out"
    );
}

/// With a live fan-out driver a credential-bound row registers
/// rotation-bound: activation returns once the fan-out has reread its
/// credentials and the row serves; rebinding the row to another credential
/// moves its rotation binding there; a credential that is gone stops the row
/// and unbinds it.
#[cfg(feature = "rotation")]
#[tokio::test]
async fn with_a_live_fanout_a_row_serves_once_its_credentials_are_reread() {
    let mut fixture = Fixture::new();
    let _driver = fixture.start_fanout();
    let first = CredentialId::new();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, first.to_string().as_str())],
        )
        .await;
    fixture.resolver.answer(Ok((1, 1)));
    let activated = fixture.activate(resource_id, &key).await.unwrap();
    let row = fixture
        .manager
        .get_row(
            &activated.resource_key,
            &activated.scope,
            &activated.slot_identity,
        )
        .expect("registered");
    assert_eq!(
        row.phase(),
        nebula_resource::ResourcePhase::Ready,
        "activation hands out a row that serves"
    );
    assert_eq!(fixture.fanout.affected(&first).len(), 1);

    let second = CredentialId::new();
    let mut stored = fixture
        .store
        .get(&fixture.scope, &resource_id.to_string())
        .await
        .unwrap()
        .unwrap();
    stored.credential_bindings = BTreeMap::from([(AUTH_SLOT.to_owned(), second.to_string())]);
    stored.version = 1;
    fixture
        .store
        .update(&fixture.scope, stored, 0)
        .await
        .unwrap();
    assert_eq!(
        fixture.activate(resource_id, &key).await.unwrap(),
        activated,
        "the same identity re-registers in place"
    );
    assert!(
        fixture.fanout.affected(&first).is_empty(),
        "the replaced credential no longer reaches the row"
    );
    assert_eq!(fixture.fanout.affected(&second).len(), 1);

    fixture
        .resolver
        .answer(Err(CredentialSlotResolveError::NotFound));
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::Credential { .. })
    );
    assert!(
        fixture
            .manager
            .get_row(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity
            )
            .is_none(),
        "a credential that is gone stops the row"
    );
    assert!(fixture.fanout.affected(&second).is_empty());
}

/// An activation abandoned while its rotation-bound row waits for the
/// fan-out retires the registration it made: no row outlives an activation
/// that never recorded it.
#[cfg(feature = "rotation")]
#[tokio::test]
async fn an_activation_abandoned_in_the_fanout_wait_retires_its_registration() {
    let mut fixture = Fixture::new();
    fixture.activator =
        StoredResourceActivator::new(Arc::clone(&fixture.store) as Arc<dyn ResourceStore>)
            .with_activation_timeout(Duration::from_millis(300));
    let _driver = fixture.start_fanout();
    let credential = CredentialId::new();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, credential.to_string().as_str())],
        )
        .await;
    fixture.resolver.answer(Ok((1, 1)));
    // The activation's own resolve answers; the fan-out's reread never does.
    let answered = fixture.resolver.calls.load(Ordering::SeqCst);
    fixture
        .resolver
        .stall_from
        .store(answered + 1, Ordering::SeqCst);
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::TimedOut(_))
    );
    let scope = ScopeLevel::Workspace(WorkspaceId::parse(&fixture.scope.workspace_id).unwrap());
    assert!(
        fixture.manager.get_any(&key, &scope).is_none(),
        "the abandoned registration is retired"
    );
    assert!(fixture.fanout.affected(&credential).is_empty());
}

/// Without a live fan-out driver nothing would ever reread a rotation-bound
/// row's credentials, so a credential-bound row registers opted out of
/// rotation and serves at once; activation's own re-check keeps it current.
#[cfg(feature = "rotation")]
#[tokio::test]
async fn without_a_live_fanout_a_row_opts_out_of_rotation() {
    let fixture = Fixture::new();
    let credential = CredentialId::new();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, credential.to_string().as_str())],
        )
        .await;
    fixture.resolver.answer(Ok((1, 1)));
    let activated = fixture.activate(resource_id, &key).await.unwrap();
    assert_eq!(
        fixture
            .manager
            .get_row(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity
            )
            .expect("registered")
            .phase(),
        nebula_resource::ResourcePhase::Ready
    );
    assert!(fixture.fanout.affected(&credential).is_empty());
}

#[tokio::test]
async fn a_deleted_row_is_retired_from_the_manager() {
    let fixture = Fixture::new();
    let mut events = fixture.manager.subscribe_events();
    let (resource_id, key) = fixture.store_row("activation.plain", "a", &[]).await;
    fixture.activate(resource_id, &key).await.unwrap();

    fixture
        .store
        .soft_delete(&fixture.scope, &resource_id.to_string())
        .await
        .unwrap();
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::NotFound { .. })
    );
    assert_eq!(drain(&mut events), (1, 1));
}

/// One sweep re-reads a bounded batch of rows; successive sweeps reach
/// every row, and retired rows stop being tracked.
#[tokio::test]
async fn the_deletion_sweep_is_bounded_and_rotates() {
    let fixture = Fixture::new();
    let mut events = fixture.manager.subscribe_events();
    let total = RETIRE_SWEEP_BATCH + 5;
    for index in 0..total {
        let (resource_id, key) = fixture
            .store_row("activation.plain", &format!("row-{index}"), &[])
            .await;
        fixture.activate(resource_id, &key).await.unwrap();
        fixture
            .store
            .soft_delete(&fixture.scope, &resource_id.to_string())
            .await
            .unwrap();
    }
    assert_eq!(drain(&mut events), (total, 0));

    fixture
        .activator
        .retire_deleted(&fixture.context(false))
        .await;
    assert_eq!(
        drain(&mut events),
        (0, RETIRE_SWEEP_BATCH),
        "one batch per sweep"
    );
    assert_eq!(fixture.activator.rows.len(), total - RETIRE_SWEEP_BATCH);

    fixture
        .activator
        .retire_deleted(&fixture.context(false))
        .await;
    assert_eq!(drain(&mut events), (0, total - RETIRE_SWEEP_BATCH));
    assert!(
        fixture.activator.rows.is_empty(),
        "retired rows are no longer tracked"
    );
}

/// A row whose stored version failed to register is reported as failed at
/// that version until it registers or is deleted; an activation that failed
/// before reading its row leaves no tracking entry once swept.
#[tokio::test]
async fn a_failed_activation_is_tracked_until_its_row_goes() {
    let fixture = Fixture::new();
    let (resource_id, key) = fixture.store_row("activation.unknown", "a", &[]).await;
    assert!(fixture.activate(resource_id, &key).await.is_err());
    let failed = |fixture: &Fixture| {
        fixture
            .activator
            .row_states()
            .into_iter()
            .any(|state| matches!(state, RowState::Failed { .. }))
    };
    assert!(failed(&fixture), "{:?}", fixture.activator.row_states());

    fixture
        .activator
        .retire_deleted(&fixture.context(false))
        .await;
    assert!(failed(&fixture), "a live row's failure is kept");

    fixture
        .store
        .soft_delete(&fixture.scope, &resource_id.to_string())
        .await
        .unwrap();
    fixture
        .activator
        .retire_deleted(&fixture.context(false))
        .await;
    assert!(fixture.activator.rows.is_empty());

    let missing = ResourceId::new();
    assert!(fixture.activate(missing, &key).await.is_err());
    fixture
        .activator
        .retire_deleted(&fixture.context(false))
        .await;
    assert!(fixture.activator.rows.is_empty());
}

/// A policy naming an account slot the resource does not declare fails
/// activation instead of limiting the row on its own.
#[tokio::test]
async fn an_undeclared_account_slot_fails_activation() {
    let fixture = Fixture::new();
    let (resource_id, key) = fixture.store_row("activation.misnamed", "a", &[]).await;
    let error = fixture.activate(resource_id, &key).await.unwrap_err();
    std::assert_matches!(
        error,
        StoredResourceActivationError::UndeclaredAccountSlot { ref slot } if slot == "billing"
    );
}

/// A retirement the manager pushed back is retried by the next sweep,
/// unless a tracked row serves that identity again.
#[tokio::test]
async fn a_pushed_back_retirement_is_retried_by_the_sweep() {
    let fixture = Fixture::new();
    let mut events = fixture.manager.subscribe_events();
    let (stale_id, key) = fixture.store_row("activation.plain", "a", &[]).await;
    let stale = fixture.activate(stale_id, &key).await.unwrap();
    let (live_id, _) = fixture.store_row("activation.plain", "b", &[]).await;
    let live = fixture.activate(live_id, &key).await.unwrap();
    assert_eq!(drain(&mut events), (2, 0));

    // As if the manager had refused both removals: the stale row is no
    // longer tracked, the live one still is.
    fixture
        .activator
        .rows
        .remove(&(fixture.scope.clone(), stale_id));
    let pending = |row: ResourceId, activated: &ActivatedResource| PendingRetirement {
        row: (fixture.scope.clone(), row),
        stale: ActiveRow {
            version: 0,
            activated: activated.clone(),
            bindings: Vec::new(),
        },
    };
    fixture
        .activator
        .pending_retirements
        .lock()
        .unwrap()
        .extend([pending(stale_id, &stale), pending(live_id, &live)]);

    fixture
        .activator
        .retire_deleted(&fixture.context(false))
        .await;
    assert_eq!(drain(&mut events), (0, 1), "only the stale row is removed");
    assert!(
        fixture
            .activator
            .pending_retirements
            .lock()
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .manager
            .get_row(&live.resource_key, &live.scope, &live.slot_identity)
            .is_some(),
        "a row served again is not removed"
    );
}

/// Every activation re-checks a credential-bound row's credentials against
/// the credential store: unchanged, the registration is reused; refreshed,
/// the row registers again; unreachable for now, it keeps serving; gone, it
/// is retired and fails.
#[tokio::test]
async fn activation_follows_credential_changes_without_a_definition_change() {
    let fixture = Fixture::new();
    let mut events = fixture.manager.subscribe_events();
    let credential = CredentialId::new().to_string();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, credential.as_str())],
        )
        .await;
    fixture.resolver.answer(Ok((1, 1)));
    let activated = fixture.activate(resource_id, &key).await.unwrap();
    assert_eq!(drain(&mut events).0, 1);

    fixture.activate(resource_id, &key).await.unwrap();
    assert_eq!(
        drain(&mut events),
        (0, 0),
        "unchanged credentials reuse the row"
    );
    assert_eq!(fixture.resolver.calls.load(Ordering::SeqCst), 2);

    fixture.resolver.answer(Ok((2, 1)));
    assert_eq!(
        fixture.activate(resource_id, &key).await.unwrap(),
        activated
    );
    assert_eq!(
        drain(&mut events).0,
        1,
        "a refreshed credential registers again"
    );

    fixture
        .resolver
        .answer(Err(CredentialSlotResolveError::Unavailable));
    assert_eq!(
        fixture.activate(resource_id, &key).await.unwrap(),
        activated
    );
    assert_eq!(
        drain(&mut events),
        (0, 0),
        "a transient failure keeps the row"
    );

    fixture
        .resolver
        .answer(Err(CredentialSlotResolveError::NotFound));
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::Credential {
            source: CredentialSlotResolveError::NotFound,
            ..
        })
    );
    assert!(
        fixture
            .manager
            .get_row(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity
            )
            .is_none(),
        "a revoked credential's row stops serving"
    );
    assert!(
        fixture
            .activator
            .row_states()
            .iter()
            .any(|state| matches!(state, RowState::Failed { .. }))
    );
}

/// An activation that times out after reading its row records that
/// version as failed, so status reports it failed rather than inactive.
#[tokio::test(start_paused = true)]
async fn a_timed_out_activation_is_recorded_as_failed() {
    let mut fixture = Fixture::new();
    fixture.activator =
        StoredResourceActivator::new(Arc::clone(&fixture.store) as Arc<dyn ResourceStore>)
            .with_activation_timeout(Duration::from_secs(1));
    let credential = CredentialId::new().to_string();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, credential.as_str())],
        )
        .await;
    fixture.resolver.stall.store(true, Ordering::SeqCst);
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::TimedOut(_))
    );
    assert!(
        fixture
            .activator
            .row_states()
            .iter()
            .any(|state| matches!(state, RowState::Failed { .. })),
        "{:?}",
        fixture.activator.row_states()
    );
}

/// With a live rotation fan-out the durable check behaves the same: a
/// refreshed credential registers the row again and a revoked one stops it,
/// whether or not the fan-out saw the change.
#[cfg(feature = "rotation")]
#[tokio::test]
async fn with_a_live_fanout_the_durable_check_still_follows_credentials() {
    let mut fixture = Fixture::new();
    let _driver = fixture.start_fanout();
    let mut events = fixture.manager.subscribe_events();
    let credential = CredentialId::new().to_string();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, credential.as_str())],
        )
        .await;
    fixture.resolver.answer(Ok((1, 1)));
    let activated = fixture.activate(resource_id, &key).await.unwrap();
    fixture.resolver.answer(Ok((2, 1)));
    fixture.activate(resource_id, &key).await.unwrap();
    fixture.activate(resource_id, &key).await.unwrap();
    assert_eq!(
        drain(&mut events).0,
        2,
        "the refreshed credential registered the row again, once"
    );

    fixture
        .resolver
        .answer(Err(CredentialSlotResolveError::NotFound));
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::Credential { .. })
    );
    assert!(
        fixture
            .manager
            .get_row(
                &activated.resource_key,
                &activated.scope,
                &activated.slot_identity
            )
            .is_none(),
        "a revoke the fan-out missed still stops the row"
    );
}

/// A credential re-check that timed out is not a lasting failure: the
/// next activation that finds the row current reports it active again.
#[tokio::test(start_paused = true)]
async fn a_timed_out_recheck_clears_on_the_next_activation() {
    let mut fixture = Fixture::new();
    fixture.activator =
        StoredResourceActivator::new(Arc::clone(&fixture.store) as Arc<dyn ResourceStore>)
            .with_activation_timeout(Duration::from_secs(1));
    let credential = CredentialId::new().to_string();
    let (resource_id, key) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[(AUTH_SLOT, credential.as_str())],
        )
        .await;
    fixture.resolver.answer(Ok((1, 1)));
    fixture.activate(resource_id, &key).await.unwrap();

    fixture.resolver.stall.store(true, Ordering::SeqCst);
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::TimedOut(_))
    );
    fixture.resolver.stall.store(false, Ordering::SeqCst);
    fixture.activate(resource_id, &key).await.unwrap();
    assert!(
        fixture
            .activator
            .row_states()
            .iter()
            .all(|state| matches!(state, RowState::Active(_))),
        "{:?}",
        fixture.activator.row_states()
    );
}

#[tokio::test]
async fn one_broken_row_does_not_affect_another() {
    let fixture = Fixture::new();
    let (unknown, unknown_key) = fixture.store_row("activation.unknown", "a", &[]).await;
    let (invalid, plain_key) = fixture.store_row("activation.plain", "", &[]).await;
    let (good, _) = fixture.store_row("activation.plain", "ok", &[]).await;

    std::assert_matches!(
        fixture.activate(unknown, &unknown_key).await,
        Err(StoredResourceActivationError::UnknownKind { .. })
    );
    std::assert_matches!(
        fixture.activate(invalid, &plain_key).await,
        Err(StoredResourceActivationError::Register(_))
    );
    assert!(fixture.activate(good, &plain_key).await.is_ok());
}

/// The row's operator settings reach the registration: a valid override
/// activates, and settings the kind refuses fail that row closed instead of
/// being silently dropped.
#[tokio::test]
async fn stored_operator_settings_reach_the_registration() {
    let fixture = Fixture::new();
    let (slower, key) = fixture
        .store_row_with_settings(
            "activation.plain",
            "a",
            &[],
            None,
            Some(serde_json::json!({ "rate": { "requests": 1, "period_ms": 1000 } })),
        )
        .await;
    fixture
        .activate(slower, &key)
        .await
        .expect("a valid override activates");

    let (malformed, _) = fixture
        .store_row_with_settings(
            "activation.plain",
            "b",
            &[],
            None,
            Some(serde_json::json!({ "rate": "fast" })),
        )
        .await;
    std::assert_matches!(
        fixture.activate(malformed, &key).await,
        Err(StoredResourceActivationError::Register(_))
    );

    // The test kinds use a fixed topology, which takes no settings.
    let (tuned, _) = fixture
        .store_row_with_settings(
            "activation.plain",
            "c",
            &[],
            Some(serde_json::json!({ "max_size": 4 })),
            None,
        )
        .await;
    std::assert_matches!(
        fixture.activate(tuned, &key).await,
        Err(StoredResourceActivationError::Register(_))
    );
}

#[tokio::test]
async fn a_row_of_another_kind_than_the_binding_is_refused() {
    let fixture = Fixture::new();
    let (resource_id, _) = fixture.store_row("activation.plain", "a", &[]).await;
    std::assert_matches!(
        fixture.activate(resource_id, &Slotted::key()).await,
        Err(StoredResourceActivationError::KindMismatch { .. })
    );
}

#[tokio::test]
async fn credential_bindings_are_checked_against_the_declared_slots() {
    let fixture = Fixture::new();
    let key = Slotted::key();
    let credential = CredentialId::new().to_string();

    let (row, _) = fixture
        .store_row(
            "activation.slotted",
            "a",
            &[("auth", &credential), ("other", &credential)],
        )
        .await;
    std::assert_matches!(
        fixture.activate(row, &key).await,
        Err(StoredResourceActivationError::UndeclaredSlot { slot }) if slot == "other"
    );

    let (row, _) = fixture.store_row("activation.slotted", "a", &[]).await;
    std::assert_matches!(
        fixture.activate(row, &key).await,
        Err(StoredResourceActivationError::MissingRequiredSlot { slot }) if slot == AUTH_SLOT
    );

    let (row, _) = fixture
        .store_row("activation.slotted", "a", &[("auth", "Prod Database")])
        .await;
    std::assert_matches!(
        fixture.activate(row, &key).await,
        Err(StoredResourceActivationError::InvalidCredentialSelector { .. })
    );
    assert_eq!(fixture.resolver.calls.load(Ordering::SeqCst), 0);

    let (row, _) = fixture
        .store_row("activation.slotted", "a", &[("auth", &credential)])
        .await;
    std::assert_matches!(
        fixture.activate(row, &key).await,
        Err(StoredResourceActivationError::Credential {
            source: CredentialSlotResolveError::NotFound,
            ..
        })
    );
    assert_eq!(fixture.resolver.calls.load(Ordering::SeqCst), 1);
    std::assert_matches!(
        fixture
            .activator
            .activate(
                &fixture.context(false),
                &fixture.scope,
                row,
                &key,
                &fixture.cancel
            )
            .await,
        Err(StoredResourceActivationError::NoCredentialResolver { .. })
    );
}

#[tokio::test]
async fn a_cancelled_turn_stops_activation() {
    let fixture = Fixture::new();
    let (resource_id, key) = fixture.store_row("activation.plain", "a", &[]).await;
    fixture.cancel.cancel();
    std::assert_matches!(
        fixture.activate(resource_id, &key).await,
        Err(StoredResourceActivationError::Cancelled)
    );
}

fn binding(slot: &str, credential: CredentialId) -> BoundCredential {
    BoundCredential {
        credential_id: credential,
        slot: slot.to_owned(),
        credential_key: CredentialKey::new("auth").expect("valid credential key"),
        material: (1, 1),
    }
}

/// Rows bound to the same credentials in one tenant share a quota key; any
/// other tenant, credential set or secret gets a different one, and the key
/// never spells the credential ids.
#[test]
fn account_limit_key_is_per_tenant_credential_set_and_secret() {
    let secret = [7_u8; 32];
    let tenant = Scope::new(
        WorkspaceId::new().to_string(),
        nebula_core::OrgId::new().to_string(),
    );
    let other_tenant = Scope::new(
        WorkspaceId::new().to_string(),
        nebula_core::OrgId::new().to_string(),
    );
    let (a, b) = (CredentialId::new(), CredentialId::new());

    let key =
        account_limit_key(&secret, &tenant, &[binding("x", a), binding("y", b)], &[]).unwrap();
    assert_eq!(
        account_limit_key(&secret, &tenant, &[binding("y", b), binding("x", a)], &[]),
        Some(key.clone()),
        "slot order does not matter"
    );
    assert_ne!(
        account_limit_key(
            &secret,
            &other_tenant,
            &[binding("x", a), binding("y", b)],
            &[]
        ),
        Some(key.clone())
    );
    assert_ne!(
        account_limit_key(&secret, &tenant, &[binding("x", a)], &[]),
        Some(key.clone())
    );
    assert_ne!(
        account_limit_key(
            &[8_u8; 32],
            &tenant,
            &[binding("x", a), binding("y", b)],
            &[]
        ),
        Some(key.clone())
    );
    assert!(!key.as_str().contains(&a.to_string()));
    assert_eq!(account_limit_key(&secret, &tenant, &[], &[]), None);

    // With the account slot declared, an auxiliary credential does not
    // split the quota.
    let c = CredentialId::new();
    let account_only = account_limit_key(&secret, &tenant, &[binding("x", a)], &[]);
    assert_eq!(
        account_limit_key(
            &secret,
            &tenant,
            &[binding("x", a), binding("y", b)],
            &["x"]
        ),
        account_only
    );
    assert_eq!(
        account_limit_key(
            &secret,
            &tenant,
            &[binding("x", a), binding("y", c)],
            &["x"]
        ),
        account_only
    );
}
