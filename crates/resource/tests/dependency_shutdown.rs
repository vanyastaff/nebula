//! Retiring framework roots must release dependencies before global guard drain.

use std::{
    any::Any,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use nebula_core::ResourceKey;
use nebula_resource::{
    AcquireOptions, Error, Manager, ManagerConfig, Pooled, RegistrationSpec, Resident,
    ResidentConfig, ResourceContext, ScopeLevel, ShutdownConfig, SlotIdentity, TeardownCx,
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::{
        pooled::{BrokenCheck, PoolProvider},
        resident::ResidentProvider,
    },
};

#[derive(Clone, Debug, nebula_schema::Schema)]
struct Config;
impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        0
    }
}

type Dependencies = Vec<Box<dyn Any + Send + Sync>>;
type DestroyOrder = Arc<Mutex<Vec<usize>>>;

struct Instance {
    dependencies: Dependencies,
    value: usize,
}

#[derive(Clone, Default)]
struct ResidentNode<const ID: usize> {
    dependencies: Arc<Mutex<Dependencies>>,
    destroyed: Arc<AtomicUsize>,
    destroy_order: DestroyOrder,
}

#[async_trait::async_trait]
impl<const ID: usize> Provider for ResidentNode<ID> {
    type Config = Config;
    type Instance = Instance;
    type Topology = Resident<Self>;
    fn key() -> ResourceKey {
        ResourceKey::try_from(format!("dependency-node-{ID}")).unwrap()
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::from_key(Self::key())
    }
    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<Instance, Error> {
        Ok(Instance {
            dependencies: std::mem::take(&mut *self.dependencies.lock().unwrap()),
            value: ID,
        })
    }
    async fn destroy(&self, instance: Instance, _: TeardownCx) -> Result<(), Error> {
        self.destroy_order.lock().unwrap().push(ID);
        drop(instance.dependencies);
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
impl<const ID: usize> nebula_resource::resource::HasCredentialSlots for ResidentNode<ID> {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
    fn declares_credential_slots() -> bool {
        false
    }
}
impl<const ID: usize> ResidentProvider for ResidentNode<ID> {
    fn is_alive_sync(&self, _: &Instance) -> bool {
        true
    }
}

#[derive(Clone, Default)]
struct PooledParent {
    dependencies: Arc<Mutex<Dependencies>>,
    destroyed: Arc<AtomicUsize>,
}
nebula_resource::no_credential_slots!(PooledParent);
#[async_trait::async_trait]
impl Provider for PooledParent {
    type Config = Config;
    type Instance = Instance;
    type Topology = Pooled<Self>;
    fn key() -> ResourceKey {
        nebula_core::resource_key!("dependency-pooled-parent")
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::from_key(Self::key())
    }
    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<Instance, Error> {
        Ok(Instance {
            dependencies: std::mem::take(&mut *self.dependencies.lock().unwrap()),
            value: 10,
        })
    }
    async fn destroy(&self, instance: Instance, _: TeardownCx) -> Result<(), Error> {
        drop(instance.dependencies);
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
impl PoolProvider for PooledParent {
    fn is_broken(&self, _: &Instance) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

fn manager() -> Manager {
    Manager::with_config(
        ManagerConfig::default()
            .with_release_queue_workers(1)
            .with_retirement_queue_capacity(1),
    )
}

fn default_manager() -> Manager {
    Manager::new()
}

fn context() -> ResourceContext {
    ResourceContext::minimal(
        Default::default(),
        tokio_util::sync::CancellationToken::new(),
    )
}

fn register<const ID: usize>(manager: &Manager, dependencies: Dependencies) -> Arc<AtomicUsize> {
    register_with_order::<ID>(manager, dependencies, &Arc::default())
}

fn register_with_order<const ID: usize>(
    manager: &Manager,
    dependencies: Dependencies,
    destroy_order: &DestroyOrder,
) -> Arc<AtomicUsize> {
    let resource = ResidentNode::<ID>::default();
    *resource.dependencies.lock().unwrap() = dependencies;
    let resource = ResidentNode {
        destroy_order: Arc::clone(destroy_order),
        ..resource
    };
    let destroyed = Arc::clone(&resource.destroyed);
    manager
        .register(RegistrationSpec {
            resource,
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .unwrap();
    destroyed
}

async fn acquire<const ID: usize>(
    manager: &Manager,
) -> nebula_resource::ResourceGuard<ResidentNode<ID>> {
    manager
        .acquire_resident::<ResidentNode<ID>>(&context(), &AcquireOptions::default())
        .await
        .unwrap()
}

async fn assert_complete(manager: &Manager, destroyed: &[Arc<AtomicUsize>]) {
    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::from_secs(1))
                .with_release_queue_timeout(Duration::from_secs(1)),
        )
        .await
        .unwrap();
    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert!(report.registry_cleared);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
    for count in destroyed {
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn resident_parent_releases_same_manager_child_during_graceful_shutdown() {
    let manager = manager();
    let destroy_order = Arc::default();
    let child_destroyed = register_with_order::<0>(&manager, vec![], &destroy_order);
    let child = acquire::<0>(&manager).await;
    let parent_destroyed =
        register_with_order::<1>(&manager, vec![Box::new(child)], &destroy_order);
    let parent = acquire::<1>(&manager).await;
    assert_eq!(parent.value, 1);
    assert_eq!(
        parent.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    assert_complete(&manager, &[parent_destroyed, child_destroyed]).await;
    assert_eq!(*destroy_order.lock().unwrap(), vec![1, 0]);
}

#[tokio::test(start_paused = true)]
async fn idle_pooled_parent_releases_same_manager_child_during_graceful_shutdown() {
    let manager = manager();
    let child_destroyed = register::<0>(&manager, vec![]);
    let child = acquire::<0>(&manager).await;
    let parent = PooledParent::default();
    parent.dependencies.lock().unwrap().push(Box::new(child));
    let parent_destroyed = Arc::clone(&parent.destroyed);
    manager
        .register(RegistrationSpec {
            resource: parent,
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Pooled::new(Default::default(), 0),
            recovery_gate: None,
        })
        .unwrap();
    let release_outcome = manager
        .acquire_pooled::<PooledParent>(&context(), &AcquireOptions::default())
        .await
        .unwrap()
        .release()
        .await
        .unwrap();
    assert_eq!(release_outcome, nebula_resource::ReleaseOutcome::Completed);
    assert_eq!(
        manager
            .pool_stats::<PooledParent>(&ScopeLevel::Global)
            .await
            .unwrap()
            .idle,
        1
    );
    assert_complete(&manager, &[parent_destroyed, child_destroyed]).await;
}

async fn assert_resident_chain_order(manager: Manager) {
    let destroy_order = Arc::default();
    let leaf_destroyed = register_with_order::<0>(&manager, vec![], &destroy_order);
    let middle_destroyed = register_with_order::<1>(
        &manager,
        vec![Box::new(acquire::<0>(&manager).await)],
        &destroy_order,
    );
    let root_destroyed = register_with_order::<2>(
        &manager,
        vec![Box::new(acquire::<1>(&manager).await)],
        &destroy_order,
    );
    assert_eq!(
        acquire::<2>(&manager).await.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    assert_complete(
        &manager,
        &[root_destroyed, middle_destroyed, leaf_destroyed],
    )
    .await;
    assert_eq!(*destroy_order.lock().unwrap(), vec![2, 1, 0]);
}

#[tokio::test(start_paused = true)]
async fn resident_chain_drains_with_one_retirement_slot_and_release_worker() {
    assert_resident_chain_order(manager()).await;
}

#[tokio::test(start_paused = true)]
async fn resident_chain_default_workers_destroy_dependents_before_dependencies() {
    assert_resident_chain_order(default_manager()).await;
}

async fn assert_resident_diamond_order(manager: Manager) {
    let destroy_order = Arc::default();
    let leaf_destroyed = register_with_order::<3>(&manager, vec![], &destroy_order);
    let left_destroyed = register_with_order::<2>(
        &manager,
        vec![Box::new(acquire::<3>(&manager).await)],
        &destroy_order,
    );
    let right_destroyed = register_with_order::<1>(
        &manager,
        vec![Box::new(acquire::<3>(&manager).await)],
        &destroy_order,
    );
    let root_destroyed = register_with_order::<0>(
        &manager,
        vec![
            Box::new(acquire::<1>(&manager).await),
            Box::new(acquire::<2>(&manager).await),
        ],
        &destroy_order,
    );
    assert_eq!(
        acquire::<0>(&manager).await.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    assert_complete(
        &manager,
        &[
            root_destroyed,
            left_destroyed,
            right_destroyed,
            leaf_destroyed,
        ],
    )
    .await;
    let destroy_order = destroy_order.lock().unwrap();
    let root_position = destroy_order.iter().position(|node| *node == 0).unwrap();
    let right_position = destroy_order.iter().position(|node| *node == 1).unwrap();
    let left_position = destroy_order.iter().position(|node| *node == 2).unwrap();
    let leaf_position = destroy_order.iter().position(|node| *node == 3).unwrap();
    assert!(root_position < right_position);
    assert!(root_position < left_position);
    assert!(right_position < leaf_position);
    assert!(left_position < leaf_position);
}

#[tokio::test(start_paused = true)]
async fn resident_diamond_destroys_shared_child_once() {
    assert_resident_diamond_order(manager()).await;
}

#[tokio::test(start_paused = true)]
async fn resident_diamond_default_workers_destroy_dependencies_last() {
    assert_resident_diamond_order(default_manager()).await;
}

#[tokio::test(start_paused = true)]
async fn cancelled_drain_preserves_original_deadline_and_rejects_removal() {
    let manager = manager();
    let destroyed = register::<0>(&manager, vec![]);
    let guard = acquire::<0>(&manager).await;
    let original = ShutdownConfig::default()
        .with_drain_timeout(Duration::from_secs(10))
        .with_release_queue_timeout(Duration::from_secs(5));
    let started = tokio::time::Instant::now();
    let mut shutdown = Box::pin(manager.graceful_shutdown(original));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(6)).await;
    drop(shutdown);
    assert_eq!(
        manager
            .remove(&ResidentNode::<0>::key())
            .unwrap_err()
            .kind(),
        &nebula_resource::ErrorKind::Cancelled
    );
    assert_eq!(
        manager
            .remove_for(
                &ResidentNode::<0>::key(),
                &ScopeLevel::Global,
                &SlotIdentity::Unbound
            )
            .unwrap_err()
            .kind(),
        &nebula_resource::ErrorKind::Cancelled
    );
    let error = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::from_mins(1))
                .with_drain_timeout_policy(nebula_resource::DrainTimeoutPolicy::Force),
        )
        .await
        .unwrap_err();
    std::assert_matches!(
        error,
        nebula_resource::ShutdownError::DrainTimeout { outstanding: 1 }
    );
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert_eq!(guard.value, 0);
    assert_eq!(
        guard.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    // Expired drain is not an unconditional rejection: the late zero count advances.
    assert_complete(&manager, &[destroyed]).await;
}

#[tokio::test(start_paused = true)]
async fn removed_old_generation_and_registered_successor_retire_once() {
    let manager = manager();
    let old_destroyed = register::<0>(&manager, vec![]);
    let old_guard = acquire::<0>(&manager).await;
    manager.remove(&ResidentNode::<0>::key()).unwrap();
    let successor_destroyed = register::<0>(&manager, vec![]);
    let parent_destroyed = register::<1>(&manager, vec![Box::new(old_guard)]);
    assert_eq!(
        acquire::<0>(&manager).await.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    assert_eq!(
        acquire::<1>(&manager).await.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    assert_complete(
        &manager,
        &[old_destroyed, successor_destroyed, parent_destroyed],
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn oversized_shutdown_budgets_do_not_overflow() {
    let manager = manager();
    let destroyed = register::<0>(&manager, vec![]);
    assert_eq!(
        acquire::<0>(&manager).await.release().await.unwrap(),
        nebula_resource::ReleaseOutcome::Completed
    );
    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::MAX)
                .with_release_queue_timeout(Duration::MAX),
        )
        .await
        .unwrap();
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
}
