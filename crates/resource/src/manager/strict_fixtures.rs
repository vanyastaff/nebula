//! Shared fixtures for strict credential admission tests (per acquire and,
//! through the managed call facade, per attempt): providers whose declared
//! slots implement the projection port and pin their material epochs, a
//! counting probe, and material installed with owner-qualified metadata.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use nebula_core::{CredentialId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{
    CredentialAvailabilityObserver, CredentialGuard, CredentialGuardMetadata, CredentialKey,
    ErasedCredentialGuard, TenantScope,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub(crate) use super::credential_reads::{
    CREDENTIAL_READ_TIMEOUT,
    tests::{ScriptedObserver, seen},
};
use super::{Manager, ManagerConfig, RegistrationSpec};
use crate::{
    Bounded, Error, ErrorKind, PinSlots, PoolConfig, Pooled, Provider, Resident, ResidentConfig,
    ResourceConfig, ResourceContext, SlotCell, SlotIdentity, SlotInstallError, SlotUpdate,
    call::{OpError, SessionBinding, SessionClosed, SessionEnd, SessionProvider},
    resource::{HasCredentialSlots, ResourceMetadataDraft},
    runtime::managed::ManagedResource,
    topology::{
        BoundedProvider, PoolProvider, ResidentProvider,
        pooled::{RecycleDecision, config::WarmupStrategy},
    },
};

#[derive(Clone, nebula_schema::Schema)]
pub(crate) struct Config {
    version: u64,
}

/// The row config at `version`.
pub(crate) fn config(version: u64) -> Config {
    Config { version }
}

impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        self.version
    }
}

/// Counts provider creates and can park the next one; scripts and counts
/// the pooled fixtures' sessions.
#[derive(Default)]
pub(crate) struct Probe {
    creates: AtomicUsize,
    park_create: AtomicBool,
    pub(crate) create_entered: Notify,
    pub(crate) release_create: Notify,
    opens: AtomicUsize,
    fail_next_open: AtomicBool,
    next_close: std::sync::Mutex<Option<SessionClosed>>,
    /// What each opened session saw: the instance and the pinned slots.
    opened: std::sync::Mutex<Vec<(u64, PinnedEpochs)>>,
}

/// A transaction over a pooled fixture's `u64` instance: adds `pending` to
/// it on commit.
pub(crate) struct Tx<'c> {
    pub(crate) instance: &'c mut u64,
    pub(crate) pending: u64,
}

impl Probe {
    /// Sessions opened so far (failed opens included).
    pub(crate) fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }

    /// The instance and pinned slots each successful open saw, in order.
    pub(crate) fn opened(&self) -> Vec<(u64, PinnedEpochs)> {
        self.opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The next open fails.
    pub(crate) fn fail_next_open(&self) {
        self.fail_next_open.store(true, Ordering::SeqCst);
    }

    /// The next close reports `closed` whatever it was asked.
    pub(crate) fn close_next_with(&self, closed: SessionClosed) {
        *self
            .next_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(closed);
    }

    fn open<'c>(&self, instance: &'c mut u64, slots: &PinnedEpochs) -> Result<Tx<'c>, OpError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        if self.fail_next_open.swap(false, Ordering::SeqCst) {
            return Err(OpError::new(ErrorKind::Transient, "open refused"));
        }
        self.opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((*instance, slots.clone()));
        Ok(Tx {
            instance,
            pending: 0,
        })
    }

    fn close(&self, session: Tx<'_>, end: SessionEnd) -> SessionClosed {
        let scripted = self
            .next_close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(scripted) = scripted {
            return scripted;
        }
        match end {
            SessionEnd::Commit => {
                *session.instance += session.pending;
                SessionClosed::Committed
            },
            _ => SessionClosed::RolledBack { refused: None },
        }
    }
    pub(crate) fn creates(&self) -> usize {
        self.creates.load(Ordering::SeqCst)
    }

    /// Parks the next `create` until [`release_create`](Self::release_create).
    pub(crate) fn park_next_create(&self) {
        self.park_create.store(true, Ordering::SeqCst);
    }

    async fn create(&self) -> u64 {
        if self.park_create.swap(false, Ordering::SeqCst) {
            self.create_entered.notify_one();
            self.release_create.notified().await;
        }
        self.creates.fetch_add(1, Ordering::SeqCst) as u64
    }
}

/// Material epoch each declared slot held when a unit pinned it, in slot
/// order; `None` for an unbound or revoked slot.
pub(crate) type PinnedEpochs = Vec<(&'static str, Option<u64>)>;

/// A provider per topology with declared credential slots (`db`, and
/// optionally `cache`) that implement the projection port. `PinSlots` pins
/// each slot's material epoch ([`PinnedEpochs`]); `pin_rotations` makes a
/// pin race a rotation.
macro_rules! strict_provider {
    ($ty:ident, $key:literal, $topology:ty, [$($slot:literal),+]) => {
        #[derive(Clone)]
        pub(crate) struct $ty {
            pub(crate) probe: Arc<Probe>,
            pub(crate) db: Arc<SlotCell<CredentialGuard<u64>>>,
            pub(crate) cache: Arc<SlotCell<CredentialGuard<u64>>>,
            /// Pins left that re-store `db` while they load it, bumping its
            /// generation as a rotation landing mid-pin would;
            /// `usize::MAX` rotates on every pin.
            pub(crate) pin_rotations: Arc<AtomicUsize>,
            /// `pin_slots` calls so far.
            pub(crate) pins: Arc<AtomicUsize>,
        }

        impl $ty {
            pub(crate) fn new() -> Self {
                Self {
                    probe: Arc::default(),
                    db: Arc::new(SlotCell::empty()),
                    cache: Arc::new(SlotCell::empty()),
                    pin_rotations: Arc::default(),
                    pins: Arc::default(),
                }
            }

            fn cell(&self, slot: &str) -> Option<&SlotCell<CredentialGuard<u64>>> {
                if !Self::credential_slot_names().contains(&slot) {
                    return None;
                }
                match slot {
                    "db" => Some(&self.db),
                    "cache" => Some(&self.cache),
                    _ => None,
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
                Ok(self.probe.create().await)
            }
        }

        impl HasCredentialSlots for $ty {
            fn declares_credential_slots() -> bool {
                true
            }

            fn credential_slot_names() -> &'static [&'static str] {
                &[$($slot),+]
            }

            fn credential_slot_epoch(&self) -> u64 {
                self.db.generation()
            }

            fn supports_credential_slot_projection(&self, slot: &str) -> bool {
                self.cell(slot).is_some()
            }

            fn credential_slot_projection(
                &self,
                slot: &str,
            ) -> Option<(u64, Option<CredentialGuardMetadata>)> {
                self.cell(slot).map(SlotCell::projection_snapshot)
            }

            fn install_credential_slot_at_generation(
                &self,
                slot: &str,
                guard: ErasedCredentialGuard,
                expected_generation: u64,
            ) -> Result<SlotUpdate, SlotInstallError> {
                let cell = self.cell(slot).ok_or(SlotInstallError::UnknownSlot)?;
                let metadata = guard.metadata().clone();
                let guard = guard
                    .into_typed::<u64>()
                    .map_err(|_| SlotInstallError::CredentialTypeMismatch)?;
                cell.install_projected_at_generation(expected_generation, metadata, Arc::new(guard))
            }

            fn fence_credential_slot_at_generation(
                &self,
                slot: &str,
                expected_generation: u64,
                fence: &mut dyn FnMut(),
            ) -> Result<(), SlotInstallError> {
                self.cell(slot)
                    .ok_or(SlotInstallError::UnknownSlot)?
                    .fence_projection_at_generation(expected_generation, fence)
            }

            fn install_credential_slot(
                &self,
                slot: &str,
                guard: ErasedCredentialGuard,
            ) -> Result<SlotUpdate, SlotInstallError> {
                let cell = self.cell(slot).ok_or(SlotInstallError::UnknownSlot)?;
                let metadata = guard.metadata().clone();
                let guard = guard
                    .into_typed::<u64>()
                    .map_err(|_| SlotInstallError::CredentialTypeMismatch)?;
                cell.install_projected(metadata, Arc::new(guard))
            }

            fn revoke_credential_slot(&self, slot: &str) -> Result<SlotUpdate, SlotInstallError> {
                Ok(self.cell(slot).ok_or(SlotInstallError::UnknownSlot)?.revoke())
            }
        }

        impl PinSlots for $ty {
            type Pinned = PinnedEpochs;

            fn pin_slots(&self) -> PinnedEpochs {
                self.pins.fetch_add(1, Ordering::SeqCst);
                let rotate = self
                    .pin_rotations
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| match left {
                        0 => None,
                        usize::MAX => Some(usize::MAX),
                        left => Some(left - 1),
                    })
                    .is_ok();
                Self::credential_slot_names()
                    .iter()
                    .map(|&slot| {
                        let cell = self.cell(slot);
                        let pinned = cell
                            .and_then(SlotCell::load_material_versioned)
                            .map(|(material, _)| material);
                        if rotate && slot == "db" {
                            if let Some(current) = self.db.load() {
                                self.db.store(current);
                            }
                        }
                        (slot, pinned)
                    })
                    .collect()
            }
        }
    };
}

strict_provider!(StrictResident, "strict-resident", Resident<Self>, ["db"]);
strict_provider!(StrictPooled, "strict-pooled", Pooled<Self>, ["db"]);
strict_provider!(StrictBounded, "strict-bounded", Bounded<Self>, ["db"]);
strict_provider!(
    StrictTwoSlot,
    "strict-two-slot",
    Resident<Self>,
    ["db", "cache"]
);
strict_provider!(RotatingPin, "strict-rotating-pin", Resident<Self>, ["db"]);

impl ResidentProvider for StrictResident {}
impl ResidentProvider for StrictTwoSlot {}
impl ResidentProvider for RotatingPin {}
impl BoundedProvider for StrictBounded {}

strict_provider!(
    StrictPooledSession,
    "strict-pooled-session",
    Pooled<Self>,
    ["db"]
);

impl PoolProvider for StrictPooled {
    async fn recycle(&self, _: &u64, _: &crate::InstanceMetrics) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
    }
}

impl PoolProvider for StrictPooledSession {
    async fn recycle(&self, _: &u64, _: &crate::InstanceMetrics) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
    }
}

/// Connection-bound sessions (the default binding) over the probe's script.
impl SessionProvider for StrictPooled {
    type Session<'c> = Tx<'c>;

    async fn open<'c>(
        &'c self,
        instance: &'c mut u64,
        slots: &'c PinnedEpochs,
    ) -> Result<Tx<'c>, OpError> {
        self.probe.open(instance, slots)
    }

    async fn close<'c>(&'c self, session: Tx<'c>, end: SessionEnd) -> SessionClosed {
        self.probe.close(session, end)
    }
}

/// Session-bound sessions: any healthy instance may host one.
impl SessionProvider for StrictPooledSession {
    type Session<'c> = Tx<'c>;
    const BINDING: SessionBinding = SessionBinding::Session;

    async fn open<'c>(
        &'c self,
        instance: &'c mut u64,
        slots: &'c PinnedEpochs,
    ) -> Result<Tx<'c>, OpError> {
        self.probe.open(instance, slots)
    }

    async fn close<'c>(&'c self, session: Tx<'c>, end: SessionEnd) -> SessionClosed {
        self.probe.close(session, end)
    }
}

/// Declares a slot but does not implement the projection port.
#[derive(Clone)]
pub(crate) struct ProjectionlessRow;

#[async_trait::async_trait]
impl Provider for ProjectionlessRow {
    type Config = Config;
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("strict-projectionless")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("strict-projectionless"),
            "",
        )
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<u64, Error> {
        Ok(0)
    }
}

impl HasCredentialSlots for ProjectionlessRow {
    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }

    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl ResidentProvider for ProjectionlessRow {}

/// A row with no credential slots.
#[derive(Clone)]
pub(crate) struct UnboundRow(pub(crate) Arc<Probe>);

#[async_trait::async_trait]
impl Provider for UnboundRow {
    type Config = Config;
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("strict-unbound")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("strict-unbound"), "")
    }

    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<u64, Error> {
        Ok(self.0.creates.fetch_add(1, Ordering::SeqCst) as u64)
    }
}

crate::no_credential_slots!(UnboundRow);

impl ResidentProvider for UnboundRow {}

pub(crate) fn tenant() -> SlotIdentity {
    SlotIdentity::from_bindings([("db", "tenant-a")])
}

/// The credential owner every installed projection names.
pub(crate) fn owner() -> TenantScope {
    TenantScope::new("org", "workspace")
}

/// The credential every projection in these tests names.
pub(crate) fn credential_id() -> CredentialId {
    static ID: OnceLock<CredentialId> = OnceLock::new();
    *ID.get_or_init(CredentialId::new)
}

/// A second credential, bound to the `cache` slot of [`StrictTwoSlot`].
pub(crate) fn cache_credential_id() -> CredentialId {
    static ID: OnceLock<CredentialId> = OnceLock::new();
    *ID.get_or_init(CredentialId::new)
}

pub(crate) fn credential_key() -> CredentialKey {
    "oauth".parse().expect("credential key")
}

/// Owner-qualified metadata of material `material` at use revision
/// `admission` for `credential_id`.
pub(crate) fn metadata_for(
    credential_id: CredentialId,
    material: u64,
    admission: u64,
) -> CredentialGuardMetadata {
    CredentialGuardMetadata::new(credential_id, credential_key(), material, material)
        .with_admission_epoch(admission)
        .with_scope(owner())
}

/// The projected guard of [`credential_id`] at `(material, admission)`.
pub(crate) fn guard(material: u64, admission: u64) -> ErasedCredentialGuard {
    ErasedCredentialGuard::from_typed(
        CredentialGuard::new(material),
        metadata_for(credential_id(), material, admission),
    )
}

/// Installs material directly into `cell`, as activation did before.
pub(crate) fn bind(
    cell: &SlotCell<CredentialGuard<u64>>,
    credential_id: CredentialId,
    material: u64,
    admission: u64,
) {
    let _update = cell
        .install_projected(
            metadata_for(credential_id, material, admission),
            Arc::new(CredentialGuard::new(material)),
        )
        .expect("install");
}

pub(crate) fn pool_config() -> PoolConfig {
    PoolConfig {
        min_size: 2,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        warmup: WarmupStrategy::None,
        maintenance_interval: std::time::Duration::from_hours(1),
        ..PoolConfig::default()
    }
}

pub(crate) fn context() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

/// A manager whose credential-bound rows read through `observer`.
pub(crate) fn strict_manager(
    observer: Arc<dyn CredentialAvailabilityObserver>,
    metrics: &Arc<nebula_metrics::MetricsRegistry>,
) -> Manager {
    Manager::with_config(
        ManagerConfig::default()
            .with_credential_observer(observer)
            .with_metrics_registry(Arc::clone(metrics)),
    )
}

pub(crate) fn register<R>(manager: &Manager, resource: R, topology: R::Topology) -> Result<R, Error>
where
    R: Provider<Config = Config> + Clone,
{
    manager.register(RegistrationSpec {
        resource: resource.clone(),
        config: Config { version: 1 },
        scope: ScopeLevel::Global,
        slot_identity: tenant(),
        topology,
        recovery_gate: None,
        rate_limit: None,
    })?;
    Ok(resource)
}

pub(crate) fn resident(manager: &Manager) -> StrictResident {
    register(
        manager,
        StrictResident::new(),
        Resident::new(ResidentConfig::default()),
    )
    .expect("register")
}

pub(crate) fn row<R: Provider>(manager: &Manager) -> Arc<ManagedResource<R>> {
    manager
        .lookup_any_for_slot_identity_structural(&R::key(), &ScopeLevel::Global, &tenant())
        .expect("row is registered")
        .as_any_arc()
        .downcast::<ManagedResource<R>>()
        .expect("row type")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use nebula_credential::{CredentialAvailability, CredentialAvailabilityObserver};

    use super::{
        PinSlots, RotatingPin, ScriptedObserver, StrictResident, StrictTwoSlot, bind,
        cache_credential_id, context, credential_id, resident, seen, strict_manager, tenant,
    };
    use crate::AcquireOptions;

    #[test]
    fn strict_rows_pin_each_slot_material_epoch() {
        let row = StrictTwoSlot::new();
        bind(&row.db, credential_id(), 3, 1);
        assert_eq!(row.pin_slots(), vec![("db", Some(3)), ("cache", None)]);
        bind(&row.cache, cache_credential_id(), 7, 1);
        assert_eq!(row.pin_slots(), vec![("db", Some(3)), ("cache", Some(7))]);
        assert_eq!(row.pins.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_rotating_pin_moves_the_generation_only_while_it_rotates() {
        let row = RotatingPin::new();
        bind(&row.db, credential_id(), 1, 1);
        let before = row.db.generation();

        row.pin_rotations.store(1, Ordering::SeqCst);
        assert_eq!(row.pin_slots(), vec![("db", Some(1))]);
        assert_eq!(row.db.generation(), before + 1, "the pin rotated once");
        let _ = row.pin_slots();
        assert_eq!(row.db.generation(), before + 1, "then pins are quiet");

        row.pin_rotations.store(usize::MAX, Ordering::SeqCst);
        let _ = row.pin_slots();
        let _ = row.pin_slots();
        assert_eq!(row.db.generation(), before + 3, "every pin rotates");
        assert_eq!(row.db.material_epoch(), Some(1), "material never moves");
    }

    #[tokio::test]
    async fn a_strict_acquire_reads_through_the_scripted_observer() {
        let observer = ScriptedObserver::answering(seen(1, 1, CredentialAvailability::Available));
        let manager = strict_manager(
            Arc::clone(&observer) as Arc<dyn CredentialAvailabilityObserver>,
            &Arc::default(),
        );
        let resource = resident(&manager);
        bind(&resource.db, credential_id(), 1, 1);

        let guard = manager
            .acquire_for_identity::<StrictResident>(
                &context(),
                &AcquireOptions::default(),
                &tenant(),
            )
            .await
            .expect("admitted");
        drop(guard);
        assert_eq!(observer.calls(), 1);
    }
}
