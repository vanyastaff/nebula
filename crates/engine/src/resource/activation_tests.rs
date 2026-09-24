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

const AUTH_SLOT: &str = "auth";

#[derive(Clone)]
struct Slotted;

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
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &[AUTH_SLOT]
    }
}

impl resident::ResidentProvider for Slotted {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

/// Refuses every credential and counts the attempts.
#[derive(Default)]
struct RefusingResolver {
    calls: AtomicUsize,
}

impl CredentialSlotResolver for RefusingResolver {
    fn resolve_slot<'a>(
        &'a self,
        _scope: &'a TenantScope,
        _credential_id: CredentialId,
        _expected_key: CredentialKey,
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(CredentialSlotResolveError::NotFound) })
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
                || Slotted,
                nebula_resource::topology::fixed(|| {
                    Resident::<Slotted>::new(resident::config::Config::default())
                }),
            )),
        )
        .expect("slotted resource admits");
    registrars
}

struct Fixture {
    store: Arc<InMemoryResourceStore>,
    activator: StoredResourceActivator,
    registrars: ResourceActivatorRegistry,
    manager: Manager,
    expr_engine: ExpressionEngine,
    resolver: RefusingResolver,
    scope: Scope,
    cancel: CancellationToken,
    #[cfg(feature = "rotation")]
    fanout: nebula_resource::ResourceFanoutIndex,
}

impl Fixture {
    fn new() -> Self {
        let store = Arc::new(InMemoryResourceStore::new());
        Self {
            activator: StoredResourceActivator::new(Arc::clone(&store) as Arc<dyn ResourceStore>),
            store,
            registrars: registrars(),
            manager: Manager::new(),
            expr_engine: ExpressionEngine::with_cache_size(16),
            resolver: RefusingResolver::default(),
            scope: Scope::new(
                WorkspaceId::new().to_string(),
                nebula_core::OrgId::new().to_string(),
            ),
            cancel: CancellationToken::new(),
            #[cfg(feature = "rotation")]
            fanout: nebula_resource::ResourceFanoutIndex::new(),
        }
    }

    fn context(&self, with_credentials: bool) -> ActivationContext<'_> {
        ActivationContext {
            registrars: &self.registrars,
            manager: &self.manager,
            credentials: with_credentials.then_some(&self.resolver as &dyn CredentialSlotResolver),
            expr_engine: &self.expr_engine,
            #[cfg(feature = "rotation")]
            fanout: Some(&self.fanout),
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

/// A retired row leaves the credential-rotation index too, so a later
/// refresh of its credentials is not dispatched to a row that is gone.
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

/// Re-registering a row under the same identity with a different
/// credential releases the previous registration's rotation references: the
/// replaced credential no longer reaches the row, and a credential both
/// registrations bound keeps the new one's reference.
#[cfg(feature = "rotation")]
#[tokio::test]
async fn a_same_identity_reregistration_releases_replaced_credentials() {
    let fixture = Fixture::new();
    let (resource_id, key) = fixture.store_row("activation.plain", "a", &[]).await;
    let activated = fixture.activate(resource_id, &key).await.unwrap();
    let (replaced, added, kept) = (
        CredentialId::new(),
        CredentialId::new(),
        CredentialId::new(),
    );
    let bind = |credential, slot: &str| {
        fixture.fanout.bind(
            credential,
            activated.resource_key.clone(),
            activated.scope.clone(),
            slot,
            activated.slot_identity.clone(),
        );
    };
    // The previous registration bound `replaced` and `kept`; the new one
    // bound `added` and `kept` again.
    bind(replaced, "token");
    bind(kept, "other");
    bind(added, "token");
    bind(kept, "other");
    let previous = ActiveRow {
        version: 1,
        activated: activated.clone(),
        bindings: vec![(replaced, "token".to_owned()), (kept, "other".to_owned())],
    };

    release_bindings(&fixture.context(false), &previous);

    assert!(
        fixture.fanout.affected(&replaced).is_empty(),
        "the replaced credential no longer reaches the row"
    );
    assert_eq!(fixture.fanout.affected(&added).len(), 1);
    assert_eq!(
        fixture.fanout.affected(&kept).len(),
        1,
        "a credential bound again keeps the new registration's reference"
    );
    release_bindings(
        &fixture.context(false),
        &ActiveRow {
            bindings: vec![(kept, "other".to_owned())],
            ..previous
        },
    );
    assert!(fixture.fanout.affected(&kept).is_empty());
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

fn binding(slot: &str, credential: CredentialId) -> SlotBinding {
    SlotBinding {
        slot_name: slot.to_owned(),
        credential_key: CredentialKey::new("auth").expect("valid credential key"),
        credential_id: Some(credential),
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

    let key = account_limit_key(&secret, &tenant, &[binding("x", a), binding("y", b)]).unwrap();
    assert_eq!(
        account_limit_key(&secret, &tenant, &[binding("y", b), binding("x", a)]),
        Some(key.clone()),
        "slot order does not matter"
    );
    assert_ne!(
        account_limit_key(&secret, &other_tenant, &[binding("x", a), binding("y", b)]),
        Some(key.clone())
    );
    assert_ne!(
        account_limit_key(&secret, &tenant, &[binding("x", a)]),
        Some(key.clone())
    );
    assert_ne!(
        account_limit_key(&[8_u8; 32], &tenant, &[binding("x", a), binding("y", b)]),
        Some(key.clone())
    );
    assert!(!key.as_str().contains(&a.to_string()));
    assert_eq!(account_limit_key(&secret, &tenant, &[]), None);
}
