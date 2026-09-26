//! Fan-out reconciliation of credential availability: a same-material block
//! suspends a bound row, a usable credential reopens it, and a material
//! advance installs (and reopens) through the ordinary projection path.
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{CredentialKey, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{
    Capabilities, CredentialAvailability, CredentialAvailabilityObservation,
    CredentialAvailabilityObserver, CredentialBlock, CredentialEvent, CredentialGuard,
    CredentialGuardMetadata, CredentialId, CredentialObserveError, CredentialOperationKind,
    CredentialSlotResolveError, CredentialSlotResolver, ErasedCredentialGuard, ReauthReason,
    TenantScope,
};
use nebula_eventbus::EventBus;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::{Bind, ResourceFanoutDriver, ResourceFanoutIndex, orchestrator::ScanHint};
use crate::{
    AcquireOptions, CredentialUnavailableReason, Error, ErrorKind, Manager, Provider,
    RegistrationSpec, Resident, ResidentConfig, ResourceConfig, ResourceContext, ResourceEvent,
    ResourceGuard, SlotCell, SlotIdentity, SlotInstallError, SlotUpdate,
    resource::{HasCredentialSlots, ResourceMetadataDraft},
    topology::ResidentProvider,
};

const WAKE: Duration = Duration::from_secs(5);

#[derive(Clone, nebula_schema::Schema)]
struct Config;

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct Bound {
    slot: Arc<SlotCell<CredentialGuard<u64>>>,
    creates: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
}

impl Bound {
    fn new() -> Self {
        Self {
            slot: Arc::new(SlotCell::empty()),
            creates: Arc::default(),
            refreshes: Arc::default(),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Bound {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("fanout-suspension")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("fanout-suspension"), "")
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<(), Error> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn on_credential_refresh(&self, _: &str, (): &()) -> Result<(), Error> {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl HasCredentialSlots for Bound {
    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }

    fn credential_slot_epoch(&self) -> u64 {
        self.slot.generation()
    }

    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<CredentialGuardMetadata>)> {
        (slot == "db").then(|| self.slot.projection_snapshot())
    }

    fn fence_credential_slot_at_generation(
        &self,
        slot: &str,
        expected_generation: u64,
        fence: &mut dyn FnMut(),
    ) -> Result<(), SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        self.slot
            .fence_projection_at_generation(expected_generation, fence)
    }

    fn install_credential_slot_at_generation(
        &self,
        slot: &str,
        guard: ErasedCredentialGuard,
        expected_generation: u64,
    ) -> Result<SlotUpdate, SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<u64>()
            .map_err(|_| SlotInstallError::CredentialTypeMismatch)?;
        self.slot
            .install_projected_at_generation(expected_generation, metadata, Arc::new(guard))
    }

    fn install_credential_slot(
        &self,
        slot: &str,
        guard: ErasedCredentialGuard,
    ) -> Result<SlotUpdate, SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        let metadata = guard.metadata().clone();
        let guard = guard
            .into_typed::<u64>()
            .map_err(|_| SlotInstallError::CredentialTypeMismatch)?;
        self.slot.install_projected(metadata, Arc::new(guard))
    }

    fn revoke_credential_slot(&self, slot: &str) -> Result<SlotUpdate, SlotInstallError> {
        if slot != "db" {
            return Err(SlotInstallError::UnknownSlot);
        }
        Ok(self.slot.revoke())
    }
}

impl ResidentProvider for Bound {}

/// What the credential answers, both to an observation and to a projection.
#[derive(Debug, Clone, Copy)]
enum Script {
    Available(u64),
    Blocked(CredentialBlock, u64),
    RefreshInFlight(u64),
    /// No live head; a projection finds the durable tombstone.
    Tombstoned,
    Unavailable,
}

/// A credential store fake that counts observations and projections.
struct ScriptedCredential {
    credential_id: CredentialId,
    key: CredentialKey,
    owner: TenantScope,
    script: Mutex<Script>,
    /// Whether it exposes the availability observer.
    observes: bool,
    observations: AtomicUsize,
    projections: AtomicUsize,
    observed: Notify,
}

impl ScriptedCredential {
    fn set(&self, script: Script) {
        *self.script.lock().expect("script lock") = script;
    }

    fn script(&self) -> Script {
        *self.script.lock().expect("script lock")
    }

    fn observations(&self) -> usize {
        self.observations.load(Ordering::SeqCst)
    }

    fn projections(&self) -> usize {
        self.projections.load(Ordering::SeqCst)
    }

    fn guard(&self, epoch: u64) -> ErasedCredentialGuard {
        ErasedCredentialGuard::from_typed(
            CredentialGuard::new(epoch),
            CredentialGuardMetadata::new(self.credential_id, self.key.clone(), epoch, epoch)
                .with_scope(self.owner.clone()),
        )
    }
}

type ResolveFuture<'a> = Pin<
    Box<dyn Future<Output = Result<ErasedCredentialGuard, CredentialSlotResolveError>> + Send + 'a>,
>;
type ObserveFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<CredentialAvailabilityObservation, CredentialObserveError>>
            + Send
            + 'a,
    >,
>;

impl CredentialSlotResolver for ScriptedCredential {
    fn resolve_slot<'a>(
        &'a self,
        _scope: &'a TenantScope,
        _credential_id: CredentialId,
        _expected_key: CredentialKey,
        _required_capabilities: Capabilities,
        _cancel: CancellationToken,
    ) -> ResolveFuture<'a> {
        self.projections.fetch_add(1, Ordering::SeqCst);
        let result = match self.script() {
            Script::Available(epoch) => Ok(self.guard(epoch)),
            Script::Blocked(CredentialBlock::ReauthRequired, _) => {
                Err(CredentialSlotResolveError::ReauthRequired)
            },
            Script::Blocked(
                CredentialBlock::OperationInFlight { operation }
                | CredentialBlock::ReconciliationRequired { operation },
                _,
            ) => Err(CredentialSlotResolveError::OperationBlocked { operation }),
            Script::Blocked(..) => Err(CredentialSlotResolveError::InvalidState),
            Script::RefreshInFlight(_) => Err(CredentialSlotResolveError::RefreshInFlight {
                retry_after: Duration::from_secs(1),
            }),
            Script::Tombstoned => Err(CredentialSlotResolveError::Revoked),
            Script::Unavailable => Err(CredentialSlotResolveError::Unavailable),
        };
        Box::pin(async move { result })
    }

    fn as_availability_observer(&self) -> Option<&dyn CredentialAvailabilityObserver> {
        self.observes
            .then_some(self as &dyn CredentialAvailabilityObserver)
    }
}

impl CredentialAvailabilityObserver for ScriptedCredential {
    fn observe_availability<'a>(
        &'a self,
        _scope: &'a TenantScope,
        _credential_id: CredentialId,
        _expected_key: CredentialKey,
        _cancel: CancellationToken,
    ) -> ObserveFuture<'a> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        let observed = |epoch, availability| {
            Ok(CredentialAvailabilityObservation::new(
                epoch,
                epoch,
                availability,
            ))
        };
        let result = match self.script() {
            Script::Available(epoch) => observed(epoch, CredentialAvailability::Available),
            Script::Blocked(block, epoch) => {
                observed(epoch, CredentialAvailability::Blocked(block))
            },
            Script::RefreshInFlight(epoch) => {
                observed(epoch, CredentialAvailability::RefreshInFlight)
            },
            Script::Tombstoned => Err(CredentialObserveError::Absent),
            Script::Unavailable => Err(CredentialObserveError::Unavailable),
        };
        self.observed.notify_one();
        Box::pin(async move { result })
    }
}

struct Fixture {
    manager: Arc<Manager>,
    index: Arc<ResourceFanoutIndex>,
    credential: Arc<ScriptedCredential>,
    resource: Bound,
}

fn identity() -> SlotIdentity {
    SlotIdentity::from_bindings([("db", "fanout-tenant")])
}

fn context() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

const REAUTH_BLOCK: CredentialBlock = CredentialBlock::ReauthRequired;
const REVOKE_IN_FLIGHT: CredentialBlock = CredentialBlock::OperationInFlight {
    operation: CredentialOperationKind::Revoke,
};

impl Fixture {
    /// A resident row bound to one credential whose material epoch 1 is
    /// installed; the credential answers `Available(1)` until scripted.
    async fn new(observes: bool) -> Self {
        let manager = Arc::new(Manager::new());
        let index = Arc::new(ResourceFanoutIndex::new());
        let resource = Bound::new();
        manager
            .register(RegistrationSpec {
                resource: resource.clone(),
                config: Config,
                scope: ScopeLevel::Global,
                slot_identity: identity(),
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register");
        let credential = Arc::new(ScriptedCredential {
            credential_id: CredentialId::new(),
            key: "oauth".parse().expect("credential key"),
            owner: TenantScope::new("org-fanout", "workspace-fanout"),
            script: Mutex::new(Script::Available(1)),
            observes,
            observations: AtomicUsize::new(0),
            projections: AtomicUsize::new(0),
            observed: Notify::new(),
        });
        let fixture = Self {
            manager,
            index,
            credential,
            resource,
        };
        drop(fixture.acquire().await.expect("warm the resident master"));
        let _installed = fixture
            .manager
            .install_and_refresh_slot_for_identity(
                &Bound::key(),
                ScopeLevel::Global,
                "db",
                &identity(),
                fixture.credential.guard(1),
            )
            .await
            .expect("install material 1");
        fixture.index.bind_with_context(
            fixture.credential.credential_id,
            Bind {
                resource_key: Bound::key(),
                scope: ScopeLevel::Global,
                slot_name: "db".to_owned(),
                slot_identity: identity(),
            },
            fixture.credential.owner.clone(),
            fixture.credential.key.clone(),
        );
        fixture
    }

    async fn acquire(&self) -> Result<ResourceGuard<Bound>, Error> {
        self.manager
            .acquire_for_identity::<Bound>(&context(), &AcquireOptions::default(), &identity())
            .await
    }

    /// One periodic scan. Its outcome counts are not what these tests pin:
    /// they assert the row's gate afterwards.
    async fn scan(&self) {
        let _outcome = self
            .index
            .reconcile_material(
                &self.manager,
                self.credential.as_ref(),
                None,
                ScanHint::Availability,
            )
            .await;
    }

    fn suspension(&self) -> Option<crate::CredentialSuspension> {
        self.manager
            .get_row(&Bound::key(), &ScopeLevel::Global, &identity())
            .expect("row")
            .credential_suspension()
    }

    fn gate_epoch(&self) -> crate::CredentialGateTicket {
        self.manager
            .credential_gate_ticket(&Bound::key(), &ScopeLevel::Global, &identity())
            .expect("ticket")
    }
}

fn assert_refused(
    result: Result<ResourceGuard<Bound>, Error>,
    reason: CredentialUnavailableReason,
) {
    let error = result.expect_err("a suspended row refuses acquires");
    assert!(
        matches!(error.kind(), ErrorKind::CredentialUnavailable { reason: got } if *got == reason),
        "expected CredentialUnavailable({reason:?}), got {error:?}"
    );
}

#[tokio::test]
async fn an_observed_reauth_block_suspends_without_decrypting() {
    let fixture = Fixture::new(true).await;
    let projections = fixture.credential.projections();
    fixture.credential.set(Script::Blocked(REAUTH_BLOCK, 1));
    fixture.scan().await;
    assert_eq!(
        fixture.suspension().and_then(|s| s.reason_for("db")),
        Some(CredentialUnavailableReason::ReauthRequired)
    );
    assert_refused(
        fixture.acquire().await,
        CredentialUnavailableReason::ReauthRequired,
    );
    assert_eq!(
        fixture.credential.projections(),
        projections,
        "a head-only observation never projects"
    );
}

#[tokio::test]
async fn a_refresh_in_flight_storm_never_suspends() {
    let fixture = Fixture::new(true).await;
    let projections = fixture.credential.projections();
    let epoch = fixture.gate_epoch();
    let mut events = fixture.manager.subscribe_events();
    fixture.credential.set(Script::RefreshInFlight(1));
    for _ in 0..50 {
        fixture.scan().await;
    }
    assert!(fixture.suspension().is_none());
    assert_eq!(
        fixture.gate_epoch(),
        epoch,
        "no suspension advanced the gate"
    );
    assert_eq!(fixture.credential.observations(), 50);
    assert_eq!(fixture.credential.projections(), projections);
    while let Some(event) = events.try_recv() {
        assert!(
            !matches!(event, ResourceEvent::CredentialSuspended { .. }),
            "unexpected {event:?}"
        );
    }
    drop(fixture.acquire().await.expect("the row keeps serving"));
}

#[tokio::test]
async fn a_revoke_in_flight_suspends_and_its_settlement_reopens() {
    let fixture = Fixture::new(true).await;
    let old = fixture.acquire().await.expect("acquire");
    fixture.credential.set(Script::Blocked(REVOKE_IN_FLIGHT, 1));
    fixture.scan().await;
    assert!(old.is_closing(), "a lease admitted before the block closes");
    assert_refused(
        fixture.acquire().await,
        CredentialUnavailableReason::OperationBlocked,
    );

    // The revoke did not apply (the provider reported it not applied, say):
    // the credential is usable again at the same material.
    fixture.credential.set(Script::Available(1));
    fixture.scan().await;
    assert!(fixture.suspension().is_none());
    let new = fixture.acquire().await.expect("the row serves again");
    assert!(!new.is_closing());
    assert!(old.is_closing(), "the old lease stays closed");
    assert!(new.admission().seq() > old.admission().seq());
    assert_eq!(
        fixture.resource.creates.load(Ordering::SeqCst),
        1,
        "reopen reuses the resident master"
    );
}

#[tokio::test]
async fn a_material_advance_after_reauth_installs_and_reopens() {
    let fixture = Fixture::new(true).await;
    fixture.credential.set(Script::Blocked(REAUTH_BLOCK, 1));
    fixture.scan().await;
    assert!(fixture.suspension().is_some());

    // Reauthentication commits material 2.
    fixture.credential.set(Script::Available(2));
    let projections = fixture.credential.projections();
    fixture.scan().await;
    assert_eq!(
        fixture.credential.projections(),
        projections + 1,
        "only the advanced material is projected"
    );
    assert!(fixture.suspension().is_none());
    let guard = fixture.acquire().await.expect("the row serves material 2");
    assert!(!guard.is_closing());
    let installed = fixture
        .resource
        .slot
        .projection_snapshot()
        .1
        .expect("installed")
        .material_epoch();
    assert_eq!(installed, 2);
}

#[tokio::test]
async fn a_tombstone_after_reauth_takes_the_revoke_path() {
    let fixture = Fixture::new(true).await;
    fixture.credential.set(Script::Blocked(REAUTH_BLOCK, 1));
    fixture.scan().await;
    fixture.credential.set(Script::Tombstoned);
    fixture.scan().await;
    let error = fixture.acquire().await.expect_err("revoked");
    assert_eq!(*error.kind(), ErrorKind::Revoked, "taint wins: {error:?}");
}

#[tokio::test]
async fn an_unavailable_store_changes_no_gate() {
    let fixture = Fixture::new(true).await;
    fixture.credential.set(Script::Unavailable);
    let epoch = fixture.gate_epoch();
    fixture.scan().await;
    assert!(fixture.suspension().is_none());
    assert_eq!(fixture.gate_epoch(), epoch);
    drop(fixture.acquire().await.expect("an outage does not suspend"));

    fixture.credential.set(Script::Blocked(REAUTH_BLOCK, 1));
    fixture.scan().await;
    fixture.credential.set(Script::Unavailable);
    fixture.scan().await;
    assert!(
        fixture.suspension().is_some(),
        "an outage does not reopen either"
    );
}

/// A row with no credential slots.
#[derive(Clone)]
struct Slotless;

#[async_trait::async_trait]
impl Provider for Slotless {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("fanout-slotless")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("fanout-slotless"), "")
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

crate::no_credential_slots!(Slotless);

impl ResidentProvider for Slotless {}

#[tokio::test]
async fn rows_without_a_binding_are_never_observed() {
    let fixture = Fixture::new(true).await;
    fixture
        .manager
        .register(RegistrationSpec {
            resource: Slotless,
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register a slot-less row");
    let before = fixture.credential.observations();
    fixture.scan().await;
    assert_eq!(
        fixture.credential.observations(),
        before + 1,
        "only the bound row is observed"
    );
}

#[tokio::test]
async fn without_an_observer_a_projection_block_suspends_the_same_way() {
    for (script, suspended) in [
        (
            Script::Blocked(REAUTH_BLOCK, 1),
            Some(CredentialUnavailableReason::ReauthRequired),
        ),
        (
            Script::Blocked(REVOKE_IN_FLIGHT, 1),
            Some(CredentialUnavailableReason::OperationBlocked),
        ),
        (Script::RefreshInFlight(1), None),
    ] {
        let fixture = Fixture::new(false).await;
        fixture.credential.set(script);
        fixture.scan().await;
        assert_eq!(fixture.credential.observations(), 0);
        assert_eq!(
            fixture.suspension().and_then(|s| s.reason_for("db")),
            suspended,
            "{script:?}"
        );
    }
    // And a later usable projection at the same material reopens.
    let fixture = Fixture::new(false).await;
    fixture.credential.set(Script::Blocked(REAUTH_BLOCK, 1));
    fixture.scan().await;
    fixture.credential.set(Script::Available(1));
    fixture.scan().await;
    assert!(fixture.suspension().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_reauth_event_triggers_a_targeted_availability_scan() {
    let fixture = Fixture::new(true).await;
    // A context-less binding of the same credential: an availability scan
    // must never dispatch its legacy refresh hook.
    let legacy = Bound::new();
    let legacy_identity = SlotIdentity::from_bindings([("db", "legacy-tenant")]);
    fixture
        .manager
        .register(RegistrationSpec {
            resource: legacy.clone(),
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: legacy_identity.clone(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register legacy row");
    fixture.index.bind(
        fixture.credential.credential_id,
        Bound::key(),
        ScopeLevel::Global,
        "db",
        legacy_identity,
    );

    let bus = Arc::new(EventBus::<CredentialEvent>::new(16));
    let mut events = fixture.manager.subscribe_events();
    let resolver: Arc<dyn CredentialSlotResolver> = fixture.credential.clone();
    let driver = ResourceFanoutDriver::spawn_with_resolver(
        Arc::clone(&fixture.index),
        Arc::clone(&fixture.manager),
        Some(resolver),
        Arc::clone(&bus),
        None,
    );
    // The driver's first periodic scan runs at once; let it finish before
    // the credential changes.
    tokio::time::timeout(WAKE, fixture.credential.observed.notified())
        .await
        .expect("initial scan observes the bound row");

    fixture.credential.set(Script::Blocked(REAUTH_BLOCK, 1));
    let _ = bus.emit(CredentialEvent::ReauthRequired {
        credential_id: fixture.credential.credential_id,
        reason: ReauthReason::ProviderRejected,
    });
    // Well before the next 30 s periodic scan: only the event can do this.
    let suspended = tokio::time::timeout(WAKE, async {
        loop {
            match events.recv().await {
                Some(ResourceEvent::CredentialSuspended { .. }) => break,
                Some(_) => {},
                None => panic!("event bus closed"),
            }
        }
    })
    .await;
    assert!(suspended.is_ok(), "the event drives a targeted scan");
    assert_eq!(
        legacy.refreshes.load(Ordering::SeqCst),
        0,
        "an availability scan never refreshes a context-less binding"
    );
    driver.abort();
}
