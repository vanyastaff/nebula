//! Shared fixtures for strict per-acquire credential admission tests:
//! providers whose declared slots implement the projection port, a counting
//! probe, and material installed with owner-qualified metadata.

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

use super::{Manager, ManagerConfig, RegistrationSpec};
use crate::{
    Bounded, Error, PoolConfig, Pooled, Provider, Resident, ResidentConfig, ResourceConfig,
    ResourceContext, SlotCell, SlotIdentity, SlotInstallError, SlotUpdate,
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

/// Counts provider creates and can park the next one.
#[derive(Default)]
pub(crate) struct Probe {
    creates: AtomicUsize,
    park_create: AtomicBool,
    pub(crate) create_entered: Notify,
    pub(crate) release_create: Notify,
}

impl Probe {
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

/// A provider per topology with declared credential slots (`db`, and
/// optionally `cache`) that implement the projection port.
macro_rules! strict_provider {
    ($ty:ident, $key:literal, $topology:ty, [$($slot:literal),+]) => {
        #[derive(Clone)]
        pub(crate) struct $ty {
            pub(crate) probe: Arc<Probe>,
            pub(crate) db: Arc<SlotCell<CredentialGuard<u64>>>,
            pub(crate) cache: Arc<SlotCell<CredentialGuard<u64>>>,
        }

        impl $ty {
            pub(crate) fn new() -> Self {
                Self {
                    probe: Arc::default(),
                    db: Arc::new(SlotCell::empty()),
                    cache: Arc::new(SlotCell::empty()),
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

impl ResidentProvider for StrictResident {}
impl ResidentProvider for StrictTwoSlot {}
impl BoundedProvider for StrictBounded {}

impl PoolProvider for StrictPooled {
    async fn recycle(&self, _: &u64, _: &crate::InstanceMetrics) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Keep)
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
