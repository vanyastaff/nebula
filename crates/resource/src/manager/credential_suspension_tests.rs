//! Credential suspension: a bound credential denying use at its current
//! material stops a row admitting work, closes every lease admitted before,
//! and keeps the row's physical owners for reuse when it reopens.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::{CredentialId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialGuard, CredentialGuardMetadata, ErasedCredentialGuard};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::{
    CredentialGateTicket, CredentialReopenOutcome, CredentialSuspendOutcome, Manager,
    RegistrationSpec,
};
use crate::{
    AcquireOptions, Bounded, CredentialUnavailableReason, Error, ErrorKind, PoolConfig, Pooled,
    Provider, Resident, ResidentConfig, ResourceConfig, ResourceContext, ResourceEvent,
    ResourceGuard, SlotCell, SlotIdentity, SlotInstallError, SlotUpdate,
    recovery::{GateState, RecoveryGate, RecoveryGateConfig},
    resource::{HasCredentialSlots, ResourceMetadataDraft},
    runtime::managed::ManagedResource,
    topology::{
        BoundedProvider, PoolProvider, ResidentProvider,
        pooled::{RecycleDecision, config::WarmupStrategy},
    },
};

const REAUTH: CredentialUnavailableReason = CredentialUnavailableReason::ReauthRequired;
const BLOCKED: CredentialUnavailableReason = CredentialUnavailableReason::OperationBlocked;
const WAKE: Duration = Duration::from_secs(5);

#[derive(Clone, nebula_schema::Schema)]
struct Config {
    version: u64,
}

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        self.version
    }
}

/// Counts provider calls and can park the next `create`.
#[derive(Default)]
struct Probe {
    creates: AtomicUsize,
    checks: AtomicUsize,
    park_create: AtomicBool,
    create_entered: Notify,
    release_create: Notify,
}

impl Probe {
    fn creates(&self) -> usize {
        self.creates.load(Ordering::SeqCst)
    }

    fn checks(&self) -> usize {
        self.checks.load(Ordering::SeqCst)
    }

    async fn create(&self) -> Result<u64, Error> {
        if self.park_create.swap(false, Ordering::SeqCst) {
            self.create_entered.notify_one();
            self.release_create.notified().await;
        }
        Ok(self.creates.fetch_add(1, Ordering::SeqCst) as u64)
    }
}

/// One provider per topology, each with a declared `db` credential slot.
macro_rules! credential_bound_provider {
    ($ty:ident, $key:literal, $topology:ty) => {
        #[derive(Clone)]
        struct $ty {
            probe: Arc<Probe>,
            slot: Arc<SlotCell<CredentialGuard<u64>>>,
        }

        impl $ty {
            fn new() -> Self {
                Self {
                    probe: Arc::default(),
                    slot: Arc::new(SlotCell::empty()),
                }
            }
        }

        #[async_trait::async_trait]
        impl Provider for $ty {
            type Config = Config;
            type Instance = u64;
            type Topology = $topology;

            fn key() -> ResourceKey {
                resource_key!($key)
            }

            fn metadata() -> ResourceMetadataDraft {
                ResourceMetadataDraft::new(Self::key(), crate::metadata_name!($key), "")
            }

            async fn create(&self, _: &Config, _: &ResourceContext) -> Result<u64, Error> {
                self.probe.create().await
            }

            async fn check(&self, _: &u64) -> Result<(), Error> {
                self.probe.checks.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }

        impl HasCredentialSlots for $ty {
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
    };
}

credential_bound_provider!(ResidentRow, "suspension-resident", Resident<Self>);
credential_bound_provider!(PooledRow, "suspension-pooled", Pooled<Self>);
credential_bound_provider!(BoundedRow, "suspension-bounded", Bounded<Self>);

impl ResidentProvider for ResidentRow {}

impl PoolProvider for PooledRow {
    async fn recycle(&self, _: &u64, _: &crate::InstanceMetrics) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
    }
}

impl BoundedProvider for BoundedRow {}

/// A row with no credential slots at all.
#[derive(Clone)]
struct Unbound;

#[async_trait::async_trait]
impl Provider for Unbound {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("suspension-unbound")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("suspension-unbound"), "")
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

crate::no_credential_slots!(Unbound);

impl ResidentProvider for Unbound {}

fn tenant() -> SlotIdentity {
    SlotIdentity::from_bindings([("db", "tenant-a")])
}

fn context() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn register<R>(manager: &Manager, resource: R, topology: R::Topology) -> R
where
    R: Provider<Config = Config> + Clone,
{
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: Config { version: 1 },
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology,
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    resource
}

fn resident(manager: &Manager) -> ResidentRow {
    register(
        manager,
        ResidentRow::new(),
        Resident::new(ResidentConfig::default()),
    )
}

fn pool_config() -> PoolConfig {
    PoolConfig {
        min_size: 1,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        warmup: WarmupStrategy::None,
        maintenance_interval: Duration::from_hours(1),
        ..PoolConfig::default()
    }
}

fn pooled(manager: &Manager) -> PooledRow {
    register(
        manager,
        PooledRow::new(),
        Pooled::new(pool_config(), Config { version: 1 }.fingerprint()),
    )
}

fn bounded(manager: &Manager) -> BoundedRow {
    register(manager, BoundedRow::new(), Bounded::capped(2).expect("cap"))
}

fn row<R: Provider>(manager: &Manager) -> Arc<ManagedResource<R>> {
    manager
        .lookup_any_for_slot_identity_structural(&R::key(), &ScopeLevel::Global, &tenant())
        .expect("row is registered")
        .as_any_arc()
        .downcast::<ManagedResource<R>>()
        .expect("row type")
}

async fn acquire<R: Provider>(manager: &Manager) -> Result<ResourceGuard<R>, Error> {
    manager
        .acquire_for_identity::<R>(&context(), &AcquireOptions::default(), &tenant())
        .await
}

fn suspend<R: Provider>(
    manager: &Manager,
    reason: CredentialUnavailableReason,
) -> CredentialSuspendOutcome {
    manager
        .suspend_credential_row(
            &R::key(),
            &ScopeLevel::Global,
            &tenant(),
            "db",
            reason,
            None,
        )
        .expect("suspend")
}

fn ticket<R: Provider>(manager: &Manager) -> CredentialGateTicket {
    manager
        .credential_gate_ticket(&R::key(), &ScopeLevel::Global, &tenant())
        .expect("ticket")
}

fn reopen<R: Provider>(manager: &Manager, ticket: CredentialGateTicket) -> CredentialReopenOutcome {
    manager
        .reopen_credential_row(&R::key(), &ScopeLevel::Global, &tenant(), "db", ticket)
        .expect("reopen")
}

fn assert_credential_unavailable(error: &Error, reason: CredentialUnavailableReason) {
    assert!(
        matches!(
            error.kind(),
            ErrorKind::CredentialUnavailable { reason: got, .. } if *got == reason
        ),
        "expected CredentialUnavailable({reason:?}), got {error:?}"
    );
}

fn install_material(manager: &Manager, epoch: u64) -> impl Future<Output = ()> + '_ {
    let guard = ErasedCredentialGuard::from_typed(
        CredentialGuard::new(epoch),
        CredentialGuardMetadata::new(
            CredentialId::new(),
            "oauth".parse().expect("credential key"),
            epoch,
            epoch,
        ),
    );
    async move {
        let _outcome = manager
            .install_and_refresh_slot_for_identity(
                &ResidentRow::key(),
                ScopeLevel::Global,
                "db",
                &tenant(),
                guard,
            )
            .await
            .expect("install");
    }
}

// ---------------------------------------------------------------------------
// Acquire refusal and closing notices.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_suspended_row_refuses_acquires_for_every_topology() {
    let manager = Manager::new();
    resident(&manager);
    pooled(&manager);
    bounded(&manager);

    assert_eq!(
        suspend::<ResidentRow>(&manager, REAUTH),
        CredentialSuspendOutcome::Suspended
    );
    suspend::<PooledRow>(&manager, BLOCKED);
    suspend::<BoundedRow>(&manager, REAUTH);

    let error = acquire::<ResidentRow>(&manager).await.expect_err("refused");
    assert_credential_unavailable(&error, REAUTH);
    assert_eq!(error.resource_key(), Some(&ResidentRow::key()));
    assert_credential_unavailable(
        &acquire::<PooledRow>(&manager).await.expect_err("refused"),
        BLOCKED,
    );
    assert_credential_unavailable(
        &acquire::<BoundedRow>(&manager).await.expect_err("refused"),
        REAUTH,
    );
}

#[tokio::test]
async fn suspension_closes_the_held_lease_and_older_benign_generations() {
    let manager = Manager::new();
    resident(&manager);
    let first = acquire::<ResidentRow>(&manager).await.expect("acquire");
    manager
        .reload_config::<ResidentRow>(Config { version: 2 }, &ScopeLevel::Global)
        .expect("reload publishes a benign successor");
    let second = acquire::<ResidentRow>(&manager).await.expect("acquire");
    assert!(second.admission().seq() > first.admission().seq());
    let waiter = tokio::spawn(first.closing().into_closed());

    suspend::<ResidentRow>(&manager, REAUTH);

    tokio::time::timeout(WAKE, waiter)
        .await
        .expect("closing wakes a parked waiter")
        .expect("waiter task");
    assert!(first.is_closing(), "an older benign generation closes too");
    assert!(second.is_closing());
}

#[tokio::test]
async fn a_suspension_denial_does_not_trip_the_recovery_gate() {
    let manager = Manager::new();
    let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig {
        max_attempts: 3,
        base_backoff: Duration::from_secs(1),
    }));
    manager
        .register(RegistrationSpec {
            resource: ResidentRow::new(),
            config: Config { version: 1 },
            scope: ScopeLevel::Global,
            slot_identity: tenant(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: Some(Arc::clone(&gate)),
            rate_limit: None,
        })
        .expect("register");
    suspend::<ResidentRow>(&manager, REAUTH);
    for _ in 0..5 {
        acquire::<ResidentRow>(&manager).await.expect_err("refused");
    }
    assert!(matches!(gate.state(), GateState::Idle));
}

// ---------------------------------------------------------------------------
// Physical owners: no build, no probe, reuse on reopen.
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn a_suspended_pool_neither_builds_nor_probes_and_reuses_its_idle_entry() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = row::<PooledRow>(&manager);
    let lease = acquire::<PooledRow>(&manager).await.expect("acquire");
    assert_eq!(
        lease.release().await.expect("release returns the entry"),
        crate::ReleaseOutcome::Completed
    );
    assert_eq!(resource.probe.creates(), 1);
    assert_eq!(row.store.len().await, 1, "the entry is idle");

    suspend::<PooledRow>(&manager, BLOCKED);
    assert_eq!(row.refill_min_idle(&context()).await, 0);
    assert_eq!(
        manager
            .warmup_pool::<PooledRow>(&context())
            .await
            .expect("warmup of a suspended pool is a no-op"),
        0
    );
    let checks_before = resource.probe.checks();
    row.run_maintenance().await;
    assert_eq!(
        resource.probe.checks(),
        checks_before,
        "a suspended row is never health-probed"
    );
    assert_eq!(row.store.len().await, 1, "the idle entry is kept");
    assert_eq!(resource.probe.creates(), 1, "nothing was built");

    assert_eq!(
        reopen::<PooledRow>(&manager, ticket::<PooledRow>(&manager)),
        CredentialReopenOutcome::Reopened
    );
    let reused = acquire::<PooledRow>(&manager).await.expect("acquire");
    assert_eq!(*reused, 0, "the idle entry is reused");
    assert_eq!(resource.probe.creates(), 1, "reopen rebuilds nothing");
    assert!(!reused.is_closing());
}

#[tokio::test(start_paused = true)]
async fn a_suspended_pool_with_an_empty_floor_refills_nothing() {
    let manager = Manager::new();
    let resource = pooled(&manager);
    let row = row::<PooledRow>(&manager);
    suspend::<PooledRow>(&manager, REAUTH);
    assert_eq!(row.refill_min_idle(&context()).await, 0);
    assert_eq!(resource.probe.creates(), 0);
    reopen::<PooledRow>(&manager, ticket::<PooledRow>(&manager));
    assert_eq!(
        row.refill_min_idle(&context()).await,
        1,
        "the floor refills once the row admits again"
    );
}

#[tokio::test]
async fn reopen_reuses_the_resident_master_under_a_fresh_generation() {
    let manager = Manager::new();
    let resource = resident(&manager);
    let old = acquire::<ResidentRow>(&manager).await.expect("acquire");
    let stale = ticket::<ResidentRow>(&manager);
    suspend::<ResidentRow>(&manager, REAUTH);
    assert_eq!(
        reopen::<ResidentRow>(&manager, stale),
        CredentialReopenOutcome::Superseded,
        "a ticket captured before the suspension cannot reopen it"
    );
    assert_eq!(
        reopen::<ResidentRow>(&manager, ticket::<ResidentRow>(&manager)),
        CredentialReopenOutcome::Reopened
    );
    let new = acquire::<ResidentRow>(&manager).await.expect("acquire");
    assert_eq!(resource.probe.creates(), 1, "the master is reused");
    assert!(!new.is_closing());
    assert!(old.is_closing(), "the pre-suspension lease stays closed");
    assert!(new.admission().seq() > old.admission().seq());
}

// ---------------------------------------------------------------------------
// Straddling creates, reload, taint.
// ---------------------------------------------------------------------------

async fn parked_acquire(
    manager: &Arc<Manager>,
    resource: &ResidentRow,
) -> tokio::task::JoinHandle<Result<ResourceGuard<ResidentRow>, Error>> {
    resource.probe.park_create.store(true, Ordering::SeqCst);
    let task = tokio::spawn({
        let manager = Arc::clone(manager);
        async move { acquire::<ResidentRow>(&manager).await }
    });
    tokio::time::timeout(WAKE, resource.probe.create_entered.notified())
        .await
        .expect("create parks");
    task
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_straddling_a_suspension_is_refused_as_credential_unavailable() {
    let manager = Arc::new(Manager::new());
    let resource = resident(&manager);
    let parked = parked_acquire(&manager, &resource).await;
    suspend::<ResidentRow>(&manager, BLOCKED);
    resource.probe.release_create.notify_one();
    let refused = tokio::time::timeout(WAKE, parked)
        .await
        .expect("acquire completes")
        .expect("acquire task")
        .expect_err("a lease admitted before the suspension is not handed out");
    assert_credential_unavailable(&refused, BLOCKED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_straddling_suspend_and_reopen_is_still_refused() {
    let manager = Arc::new(Manager::new());
    let resource = resident(&manager);
    let parked = parked_acquire(&manager, &resource).await;
    suspend::<ResidentRow>(&manager, REAUTH);
    reopen::<ResidentRow>(&manager, ticket::<ResidentRow>(&manager));
    resource.probe.release_create.notify_one();
    let refused = tokio::time::timeout(WAKE, parked)
        .await
        .expect("acquire completes")
        .expect("acquire task")
        .expect_err("its generation closed while it was in flight");
    assert_credential_unavailable(&refused, REAUTH);
    let served = acquire::<ResidentRow>(&manager)
        .await
        .expect("the reopened row serves");
    assert!(!served.is_closing());
}

#[tokio::test]
async fn a_reload_while_suspended_does_not_reopen() {
    let manager = Manager::new();
    resident(&manager);
    suspend::<ResidentRow>(&manager, REAUTH);
    manager
        .reload_config::<ResidentRow>(Config { version: 2 }, &ScopeLevel::Global)
        .expect("reload swaps the config");
    assert_credential_unavailable(
        &acquire::<ResidentRow>(&manager).await.expect_err("refused"),
        REAUTH,
    );
    assert!(
        manager
            .get_row(&ResidentRow::key(), &ScopeLevel::Global, &tenant())
            .expect("row")
            .credential_suspension()
            .is_some()
    );
}

#[tokio::test]
async fn taint_wins_over_suspension() {
    let manager = Manager::new();
    resident(&manager);
    let held = ticket::<ResidentRow>(&manager);
    suspend::<ResidentRow>(&manager, REAUTH);
    let current = ticket::<ResidentRow>(&manager);
    let _tainted = manager
        .taint_slot_for_identity(&ResidentRow::key(), ScopeLevel::Global, "db", &tenant())
        .expect("taint");
    let error = acquire::<ResidentRow>(&manager).await.expect_err("refused");
    assert_eq!(*error.kind(), ErrorKind::Revoked);
    assert_eq!(
        reopen::<ResidentRow>(&manager, current),
        CredentialReopenOutcome::Tainted
    );
    assert_eq!(
        reopen::<ResidentRow>(&manager, held),
        CredentialReopenOutcome::Tainted
    );
    assert_eq!(
        suspend::<ResidentRow>(&manager, BLOCKED),
        CredentialSuspendOutcome::Tainted
    );
}

// ---------------------------------------------------------------------------
// Waiting, status, events, validation.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn until_accepting_fails_fast_on_a_suspended_row() {
    let manager = Manager::new();
    resident(&manager);
    suspend::<ResidentRow>(&manager, REAUTH);
    let error = tokio::time::timeout(
        WAKE,
        manager.until_accepting(&ResidentRow::key(), &ScopeLevel::Global, &tenant(), None),
    )
    .await
    .expect("returns without waiting")
    .expect_err("suspended");
    assert_credential_unavailable(&error, REAUTH);
}

#[tokio::test]
async fn status_view_and_events_report_the_suspension() {
    let manager = Manager::new();
    resident(&manager);
    let mut events = manager.subscribe_events();

    suspend::<ResidentRow>(&manager, REAUTH);
    assert_eq!(
        suspend::<ResidentRow>(&manager, REAUTH),
        CredentialSuspendOutcome::AlreadySuspended
    );
    let snapshot = manager
        .health_check::<ResidentRow>(&ScopeLevel::Global)
        .expect("snapshot");
    let suspension = snapshot.credential_suspension.expect("suspended");
    assert_eq!(suspension.reason_for("db"), Some(REAUTH));
    assert!(
        snapshot.phase.is_accepting(),
        "a suspended row keeps its phase; the suspension is reported separately"
    );

    reopen::<ResidentRow>(&manager, ticket::<ResidentRow>(&manager));
    assert!(
        manager
            .get_row(&ResidentRow::key(), &ScopeLevel::Global, &tenant())
            .expect("row")
            .credential_suspension()
            .is_none()
    );

    let mut seen = Vec::new();
    while let Some(event) = events.try_recv() {
        seen.push(event);
    }
    let suspended = seen
        .iter()
        .filter(|event| matches!(event, ResourceEvent::CredentialSuspended { .. }))
        .count();
    assert_eq!(
        suspended, 1,
        "a repeated identical denial publishes nothing"
    );
    assert!(seen.iter().any(|event| matches!(
        event,
        ResourceEvent::CredentialReopened { key } if *key == ResidentRow::key()
    )));
}

#[tokio::test]
async fn unknown_slots_are_rejected_and_slotless_rows_are_unaffected() {
    let manager = Manager::new();
    resident(&manager);
    let error = manager
        .suspend_credential_row(
            &ResidentRow::key(),
            &ScopeLevel::Global,
            &tenant(),
            "nope",
            REAUTH,
            None,
        )
        .expect_err("unknown slot");
    assert_eq!(*error.kind(), ErrorKind::Permanent);
    drop(
        acquire::<ResidentRow>(&manager)
            .await
            .expect("a rejected suspension changes nothing"),
    );

    manager
        .register(RegistrationSpec {
            resource: Unbound,
            config: Config { version: 1 },
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
    let error = manager
        .suspend_credential_row(
            &Unbound::key(),
            &ScopeLevel::Global,
            &SlotIdentity::Unbound,
            "db",
            REAUTH,
            None,
        )
        .expect_err("a slot-less row has no slot to suspend");
    assert_eq!(*error.kind(), ErrorKind::Permanent);
    drop(
        manager
            .acquire::<Unbound>(&context(), &AcquireOptions::default())
            .await
            .expect("a slot-less row keeps serving"),
    );
}

#[tokio::test]
async fn an_observation_older_than_the_installed_material_is_ignored() {
    let manager = Manager::new();
    resident(&manager);
    drop(acquire::<ResidentRow>(&manager).await.expect("warm"));
    install_material(&manager, 5).await;
    let stale = manager
        .suspend_credential_row(
            &ResidentRow::key(),
            &ScopeLevel::Global,
            &tenant(),
            "db",
            REAUTH,
            Some(4),
        )
        .expect("suspend");
    assert_eq!(stale, CredentialSuspendOutcome::StaleObservation);
    drop(
        acquire::<ResidentRow>(&manager)
            .await
            .expect("a stale denial changes nothing"),
    );
    let current = manager
        .suspend_credential_row(
            &ResidentRow::key(),
            &ScopeLevel::Global,
            &tenant(),
            "db",
            REAUTH,
            Some(5),
        )
        .expect("suspend");
    assert_eq!(current, CredentialSuspendOutcome::Suspended);
}
