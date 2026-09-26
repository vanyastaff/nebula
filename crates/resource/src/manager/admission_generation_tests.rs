//! Admission generations: which lifecycle changes close a row's generations
//! and which publish a benign successor.
use std::{sync::Arc, time::Duration};

use nebula_core::{CredentialId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{CredentialGuard, CredentialGuardMetadata, ErasedCredentialGuard};
use tokio_util::sync::CancellationToken;

use super::{
    DrainTimeoutPolicy, Manager, RegistrationSpec, ShutdownConfig, SlotDispatchOutcome,
    SlotDrainOutcome,
};
use crate::{
    AcquireOptions, Error, Provider, Resident, ResidentConfig, ResourceConfig, ResourceContext,
    ResourceGuard, SlotCell, SlotIdentity, SlotInstallError, SlotUpdate,
    resource::{HasCredentialSlots, ResourceMetadataDraft},
    runtime::managed::ManagedResource,
    topology::resident::ResidentProvider,
};

#[derive(Clone, nebula_schema::Schema)]
struct Config {
    version: u64,
}

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        self.version
    }
}

#[derive(Clone)]
struct Tenant {
    slot: Arc<SlotCell<CredentialGuard<u64>>>,
}

impl Tenant {
    fn new() -> Self {
        Self {
            slot: Arc::new(SlotCell::empty()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Tenant {
    type Config = Config;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("admission-generation")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("admission-generation"),
            "",
        )
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl HasCredentialSlots for Tenant {
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

#[async_trait::async_trait]
impl ResidentProvider for Tenant {
    fn is_alive_sync(&self, (): &()) -> bool {
        true
    }
}

fn identity(tenant: &str) -> SlotIdentity {
    SlotIdentity::from_bindings([("db", tenant)])
}

fn register(manager: &Manager, identity: &SlotIdentity, version: u64) {
    manager
        .register(RegistrationSpec {
            resource: Tenant::new(),
            config: Config { version },
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("register");
}

fn row(manager: &Manager, identity: &SlotIdentity) -> Arc<ManagedResource<Tenant>> {
    manager
        .lookup_any_for_slot_identity_structural(&Tenant::key(), &ScopeLevel::Global, identity)
        .expect("row is registered")
        .as_any_arc()
        .downcast::<ManagedResource<Tenant>>()
        .expect("row is a Tenant row")
}

fn current_seq(row: &ManagedResource<Tenant>) -> u64 {
    row.admission
        .current()
        .expect("an admitting row has a current generation")
        .seq()
}

#[tokio::test]
async fn taint_retires_the_row() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let held = row.admission.snapshot();
    let _tainted = manager
        .taint_slot_for_identity(&Tenant::key(), ScopeLevel::Global, "db", &tenant)
        .expect("taint");
    assert!(row.admission.is_retired());
    assert!(
        held.is_closed(),
        "a taint closes the generation a lease holds"
    );
    assert!(row.admission.current().is_none());
}

#[tokio::test]
async fn remove_and_remove_for_retire_the_row() {
    let manager = Manager::new();
    let (first, second) = (identity("a"), identity("b"));
    register(&manager, &first, 1);
    register(&manager, &second, 1);
    let (first_row, second_row) = (row(&manager, &first), row(&manager, &second));

    manager
        .remove_for(&Tenant::key(), &ScopeLevel::Global, &first)
        .expect("remove_for");
    assert!(first_row.admission.is_retired());
    assert!(
        !second_row.admission.is_retired(),
        "a sibling row keeps admitting"
    );

    manager.remove(&Tenant::key()).expect("remove");
    assert!(second_row.admission.is_retired());
}

#[tokio::test]
async fn replacement_leaves_the_displaced_row_open() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let displaced = row(&manager, &tenant);
    let held = displaced.admission.snapshot();
    register(&manager, &tenant, 2);
    let successor = row(&manager, &tenant);
    assert!(!Arc::ptr_eq(&displaced, &successor));
    assert!(!displaced.admission.is_retired());
    assert!(
        !held.is_closed(),
        "a same-identity replacement does not close leases on the displaced row"
    );
}

#[tokio::test]
async fn reload_publishes_a_successor_and_leaves_the_predecessor_open() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let held = row.admission.snapshot();
    manager
        .reload_config::<Tenant>(Config { version: 2 }, &ScopeLevel::Global)
        .expect("reload");
    assert!(current_seq(&row) > held.seq());
    assert!(!held.is_closed(), "a reload is benign");
}

#[tokio::test]
async fn reload_after_taint_publishes_nothing() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let _tainted = manager
        .taint_slot_for_identity(&Tenant::key(), ScopeLevel::Global, "db", &tenant)
        .expect("taint");
    manager
        .reload_config::<Tenant>(Config { version: 2 }, &ScopeLevel::Global)
        .expect("reload still swaps the config");
    assert!(row.admission.current().is_none());
    assert!(row.admission.is_retired());
}

#[tokio::test]
async fn credential_install_publishes_a_successor() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    drop(
        manager
            .acquire_resident_for_identity::<Tenant>(&context, &AcquireOptions::default(), &tenant)
            .await
            .expect("warm resident"),
    );
    let held = row.admission.snapshot();
    let guard = ErasedCredentialGuard::from_typed(
        CredentialGuard::new(7_u64),
        CredentialGuardMetadata::new(
            CredentialId::new(),
            "oauth".parse().expect("credential key"),
            2,
            2,
        ),
    );
    let outcome = manager
        .install_and_refresh_slot_for_identity(
            &Tenant::key(),
            ScopeLevel::Global,
            "db",
            &tenant,
            guard,
        )
        .await
        .expect("install and refresh");
    assert!(matches!(outcome, super::EpochRefreshOutcome::Applied(_)));
    assert!(current_seq(&row) > held.seq());
    assert!(!held.is_closed(), "a credential refresh is benign");
}

#[tokio::test]
async fn shutdown_retires_every_row() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let held = row.admission.snapshot();
    manager.shutdown();
    assert!(row.admission.is_retired());
    assert!(held.is_closed());
}

// ---------------------------------------------------------------------------
// Guards capture the generation they were admitted under.
// ---------------------------------------------------------------------------

async fn acquire(manager: &Manager, identity: &SlotIdentity) -> ResourceGuard<Tenant> {
    let context = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    manager
        .acquire_resident_for_identity::<Tenant>(&context, &AcquireOptions::default(), identity)
        .await
        .expect("acquire")
}

const WAKE: Duration = Duration::from_secs(5);

#[tokio::test]
async fn fresh_guard_is_not_closing() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let guard = acquire(&manager, &tenant).await;
    assert!(!guard.is_closing());
    assert!(!guard.closing().is_closing());
    assert_eq!(
        guard.admission().seq(),
        current_seq(&row(&manager, &tenant))
    );
}

#[tokio::test]
async fn revoke_closes_a_held_guard_and_wakes_its_waiter() {
    let manager = Arc::new(Manager::new());
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let guard = acquire(&manager, &tenant).await;
    let waiter = tokio::spawn(guard.closing().into_closed());

    let revoke = tokio::spawn({
        let manager = Arc::clone(&manager);
        let tenant = tenant.clone();
        async move {
            manager
                .revoke_slot_for_identity(&Tenant::key(), ScopeLevel::Global, "db", &tenant)
                .await
        }
    });
    tokio::time::timeout(WAKE, waiter)
        .await
        .expect("closing wakes a parked waiter")
        .expect("waiter task");
    assert!(guard.is_closing());
    tokio::time::timeout(WAKE, guard.closing().closed())
        .await
        .expect("an already-closed notice completes immediately");

    drop(guard);
    let outcome = tokio::time::timeout(WAKE, revoke)
        .await
        .expect("revoke drains once the guard is released")
        .expect("revoke task")
        .expect("revoke");
    assert!(matches!(
        outcome,
        SlotDispatchOutcome::Completed {
            drain: SlotDrainOutcome::Drained
        }
    ));
}

#[tokio::test]
async fn reload_does_not_close_an_old_guard() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let old = acquire(&manager, &tenant).await;
    manager
        .reload_config::<Tenant>(Config { version: 2 }, &ScopeLevel::Global)
        .expect("reload");
    let new = acquire(&manager, &tenant).await;
    assert!(!old.is_closing(), "a reload never interrupts a lease");
    assert!(!new.is_closing());
    assert!(new.admission().seq() > old.admission().seq());
}

#[tokio::test]
async fn closing_g1_then_publishing_g2_closes_only_old_guards() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let old = acquire(&manager, &tenant).await;
    {
        let _admission = manager
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(row.admission.close_current(), Some(old.admission().seq()));
        row.admission.publish().expect("publish G2");
    }
    let new = acquire(&manager, &tenant).await;
    assert!(old.is_closing());
    assert!(!new.is_closing());
    assert!(new.admission().seq() > old.admission().seq());
}

#[tokio::test]
async fn an_old_guard_keeps_its_own_closed_token_after_a_successor() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let row = row(&manager, &tenant);
    let old = acquire(&manager, &tenant).await;
    row.admission.close_current();
    let successor = row.admission.publish().expect("publish G2");
    // Asking again after G2 exists must not hand the old lease G2's open
    // token: the capture is immutable.
    let closing = old.closing();
    assert!(closing.is_closing());
    tokio::time::timeout(WAKE, closing.closed())
        .await
        .expect("the old lease still sees G1 closed");
    assert!(!successor.is_closed());
}

#[tokio::test]
async fn terminal_close_after_a_benign_publish_closes_every_generation() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let first = acquire(&manager, &tenant).await;
    manager
        .reload_config::<Tenant>(Config { version: 2 }, &ScopeLevel::Global)
        .expect("reload");
    let second = acquire(&manager, &tenant).await;
    assert_ne!(first.admission().seq(), second.admission().seq());
    let _tainted = manager
        .taint_slot_for_identity(&Tenant::key(), ScopeLevel::Global, "db", &tenant)
        .expect("taint");
    assert!(first.is_closing(), "G1 is still held and closes on taint");
    assert!(second.is_closing());
}

#[tokio::test]
async fn graceful_shutdown_fires_closing_at_drain_start() {
    let manager = Manager::new();
    let tenant = identity("a");
    register(&manager, &tenant, 1);
    let guard = acquire(&manager, &tenant).await;
    let closing = guard.closing();
    let shutdown = manager.graceful_shutdown(
        ShutdownConfig::default()
            .with_drain_timeout(Duration::from_millis(50))
            .with_drain_timeout_policy(DrainTimeoutPolicy::Force),
    );
    let (report, ()) = tokio::join!(shutdown, async {
        tokio::time::timeout(WAKE, closing.closed())
            .await
            .expect("closing fires while the drain waits for the guard");
    });
    let report = report.expect("forced shutdown reports incomplete drain");
    assert_eq!(report.outstanding_handles_after_drain, 1);
    assert!(guard.is_closing());
    drop(guard);
}
