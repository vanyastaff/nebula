//! `ResourceGuard`/`Handle` RAII and release-teardown integration tests for
//! nebula-resource v2: detach semantics, panic-in-release-callback isolation,
//! the release-queue teardown ceiling (hang/panic/cancellation survival),
//! pool eviction on stale fingerprint / `max_lifetime`, the recycle `Drop`
//! decision, credentialed-pool default recycle-vs-discard (ADR-0093), and
//! `ResourceGuard::release()` as an explicit awaited checkpoint.
//!
//! Split out of the former monolithic `basic_integration.rs` (pure move, no
//! test-body changes) — shared mocks/helpers live in `tests/common/mod.rs`.

mod common;

#[derive(Clone)]
struct NestedCleanupResource {
    next_id: Arc<AtomicU64>,
    children: Arc<std::sync::Mutex<std::collections::HashMap<u64, ResourceGuard<Self>>>>,
    completed: Arc<AtomicU64>,
    deferred: Arc<AtomicU64>,
    pause_child: Arc<AtomicBool>,
    child_entered: Arc<tokio::sync::Notify>,
    child_continue: Arc<tokio::sync::Notify>,
    root_barrier: Arc<std::sync::Mutex<Option<Arc<tokio::sync::Barrier>>>>,
}

#[async_trait::async_trait]
impl Provider for NestedCleanupResource {
    type Config = TestConfig;
    type Instance = u64;
    type Topology = nebula_resource::Bounded<Self>;

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("NestedCleanupResource"),
            "",
        )
    }

    fn key() -> ResourceKey {
        resource_key!("nested-cleanup")
    }

    async fn create(&self, _: &TestConfig, _: &ResourceContext) -> Result<u64, Error> {
        Ok(self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    async fn destroy(&self, instance: u64, _: nebula_resource::TeardownCx) -> Result<(), Error> {
        if instance == 0 {
            let barrier = self.root_barrier.lock().unwrap().clone();
            if let Some(barrier) = barrier {
                barrier.wait().await;
            }
        }
        if instance == 1 && self.pause_child.load(Ordering::SeqCst) {
            self.child_entered.notify_one();
            self.child_continue.notified().await;
        }
        let child = self.children.lock().unwrap().remove(&instance);
        if let Some(child) = child
            && child.release().await? == ReleaseOutcome::Deferred
        {
            self.deferred.fetch_add(1, Ordering::SeqCst);
        }
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl nebula_resource::BoundedProvider for NestedCleanupResource {}
nebula_resource::no_credential_slots!(NestedCleanupResource);

fn nested_cleanup_manager() -> (Manager, NestedCleanupResource) {
    let resource = NestedCleanupResource {
        next_id: Arc::new(AtomicU64::new(0)),
        children: Arc::default(),
        completed: Arc::new(AtomicU64::new(0)),
        deferred: Arc::new(AtomicU64::new(0)),
        pause_child: Arc::new(AtomicBool::new(false)),
        child_entered: Arc::default(),
        child_continue: Arc::default(),
        root_barrier: Arc::default(),
    };
    let manager = Manager::with_config(
        nebula_resource::ManagerConfig::default().with_release_queue_workers(1),
    );
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: nebula_resource::Bounded::unbounded(),
            recovery_gate: None,
        })
        .unwrap();
    (manager, resource)
}

#[tokio::test(start_paused = true)]
async fn nested_release_on_one_worker_completes_parent_and_child() {
    let (manager, resource) = nested_cleanup_manager();
    let parent = manager
        .acquire_bounded::<NestedCleanupResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    let child = manager
        .acquire_bounded::<NestedCleanupResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    resource.children.lock().unwrap().insert(*parent, child);
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), parent.release())
        .await
        .expect("nested release must progress without waiting for the parent's worker")
        .unwrap();
    assert_eq!(outcome, ReleaseOutcome::Completed);
    assert_eq!(resource.completed.load(Ordering::SeqCst), 2);
    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(report.dropped_release_tasks, 0);
}

async fn nested_lease(manager: &Manager) -> ResourceGuard<NestedCleanupResource> {
    manager
        .acquire_bounded::<NestedCleanupResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn cancelling_nested_release_waiter_keeps_child_hook_queue_owned() {
    let (manager, resource) = nested_cleanup_manager();
    let parent = nested_lease(&manager).await;
    let child = nested_lease(&manager).await;
    resource.children.lock().unwrap().insert(*parent, child);
    resource.pause_child.store(true, Ordering::SeqCst);
    let caller = tokio::spawn(parent.release());
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        resource.child_entered.notified(),
    )
    .await
    .unwrap();
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(resource.completed.load(Ordering::SeqCst), 0);
    resource.child_continue.notify_one();
    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(resource.completed.load(Ordering::SeqCst), 2);
    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test(start_paused = true)]
async fn cross_queue_nested_release_defers_only_accepted_child_cleanup() {
    let (first, resource) = nested_cleanup_manager();
    let (second, child_resource) = nested_cleanup_manager();
    let parent = nested_lease(&first).await;
    let child = nested_lease(&second).await;
    resource.children.lock().unwrap().insert(*parent, child);
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), parent.release())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome, ReleaseOutcome::Completed);
    assert_eq!(resource.deferred.load(Ordering::SeqCst), 1);
    let report = second
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(child_resource.completed.load(Ordering::SeqCst), 1);
    assert_eq!(report.dropped_release_tasks, 0);
    first
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn cross_queue_cleanup_cycle_defers_both_edges_without_deadlock() {
    let (first, first_resource) = nested_cleanup_manager();
    let (second, second_resource) = nested_cleanup_manager();
    let first_parent = nested_lease(&first).await;
    let first_child = nested_lease(&first).await;
    let second_parent = nested_lease(&second).await;
    let second_child = nested_lease(&second).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    *first_resource.root_barrier.lock().unwrap() = Some(Arc::clone(&barrier));
    *second_resource.root_barrier.lock().unwrap() = Some(barrier);
    first_resource
        .children
        .lock()
        .unwrap()
        .insert(*first_parent, second_child);
    second_resource
        .children
        .lock()
        .unwrap()
        .insert(*second_parent, first_child);
    let outcomes = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(first_parent.release(), second_parent.release())
    })
    .await
    .expect("cross-queue cleanup cannot form a wait cycle");
    assert_eq!(outcomes.0.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(outcomes.1.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(first_resource.deferred.load(Ordering::SeqCst), 1);
    assert_eq!(second_resource.deferred.load(Ordering::SeqCst), 1);
    let (first_report, second_report) = tokio::join!(
        first.graceful_shutdown(ShutdownConfig::default()),
        second.graceful_shutdown(ShutdownConfig::default()),
    );
    assert_eq!(first_report.unwrap().dropped_release_tasks, 0);
    assert_eq!(second_report.unwrap().dropped_release_tasks, 0);
    assert_eq!(first_resource.completed.load(Ordering::SeqCst), 2);
    assert_eq!(second_resource.completed.load(Ordering::SeqCst), 2);
}

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use common::{
    PoolTestResource, ResidentTestResource, TestConfig, idle_count, poll_until,
    pool_manager_with_metrics, register_pool, test_config, test_ctx, wait_count_at_least,
    wait_idle_count,
};
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, Manager, Pooled, RegistrationSpec, ReleaseOutcome, Resident, ResidentConfig,
    ResourceContext, ScopeLevel, ShutdownConfig, SlotIdentity,
    error::Error,
    guard::ResourceGuard,
    resource::{HasCredentialSlots, Provider, ResourceMetadataDraft},
    topology::pooled::{PoolProvider, RecycleDecision},
};

#[derive(Clone)]
struct ResidentLifecycleResource {
    next_id: Arc<AtomicU64>,
    destroyed: Arc<std::sync::Mutex<Vec<u64>>>,
    alive: Arc<AtomicBool>,
    fail_create: Arc<AtomicBool>,
    fail_destroy: Arc<AtomicBool>,
}

// A unique instance must work even for shared Resident leases.
struct UniqueResident(u64);

#[async_trait::async_trait]
impl Provider for ResidentLifecycleResource {
    type Config = TestConfig;
    type Instance = UniqueResident;
    type Topology = Resident<Self>;

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("ResidentLifecycleResource"),
            "",
        )
    }

    fn key() -> ResourceKey {
        resource_key!("resident-lifecycle")
    }

    async fn create(&self, _: &TestConfig, _: &ResourceContext) -> Result<UniqueResident, Error> {
        if self.fail_create.load(Ordering::SeqCst) {
            return Err(Error::permanent("intentional replacement failure"));
        }
        Ok(UniqueResident(self.next_id.fetch_add(1, Ordering::SeqCst)))
    }

    async fn destroy(
        &self,
        instance: UniqueResident,
        _: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroyed.lock().unwrap().push(instance.0);
        if self.fail_destroy.load(Ordering::SeqCst) {
            return Err(Error::permanent("intentional retained teardown failure"));
        }
        Ok(())
    }
}

impl nebula_resource::topology::ResidentProvider for ResidentLifecycleResource {
    fn is_alive_sync(&self, _: &UniqueResident) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

nebula_resource::no_credential_slots!(ResidentLifecycleResource);

fn resident_lifecycle_manager() -> (Manager, ResidentLifecycleResource) {
    let resource = ResidentLifecycleResource {
        next_id: Arc::new(AtomicU64::new(0)),
        destroyed: Arc::new(std::sync::Mutex::new(Vec::new())),
        alive: Arc::new(AtomicBool::new(true)),
        fail_create: Arc::new(AtomicBool::new(false)),
        fail_destroy: Arc::new(AtomicBool::new(false)),
    };
    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig {
                recreate_on_failure: true,
                ..Default::default()
            }),
            recovery_gate: None,
        })
        .unwrap();
    (manager, resource)
}

#[tokio::test]
async fn graceful_shutdown_reports_terminal_failure_and_finishes_sibling_rows() {
    let (manager, failing) = resident_lifecycle_manager();
    let sibling = ResidentLifecycleResource {
        next_id: Arc::new(AtomicU64::new(0)),
        destroyed: Arc::default(),
        alive: Arc::new(AtomicBool::new(true)),
        fail_create: Arc::new(AtomicBool::new(false)),
        fail_destroy: Arc::new(AtomicBool::new(false)),
    };
    let identity =
        SlotIdentity::Structural(Arc::from([("scope".to_owned(), "sibling".to_owned())]));
    manager
        .register(RegistrationSpec {
            resource: sibling.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: identity.clone(),
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .unwrap();
    let first_outcome = manager
        .acquire_resident_for_identity::<ResidentLifecycleResource>(
            &test_ctx(),
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .unwrap()
        .release()
        .await
        .unwrap();
    assert_eq!(first_outcome, ReleaseOutcome::Completed);
    let sibling_outcome = manager
        .acquire_resident_for_identity::<ResidentLifecycleResource>(
            &test_ctx(),
            &AcquireOptions::default(),
            &identity,
        )
        .await
        .unwrap()
        .release()
        .await
        .unwrap();
    assert_eq!(sibling_outcome, ReleaseOutcome::Completed);
    failing.fail_destroy.store(true, Ordering::SeqCst);
    let error = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap_err();
    match error {
        nebula_resource::ShutdownError::ResourceTeardownFailed { key, source } => {
            assert_eq!(key, ResidentLifecycleResource::key());
            assert_eq!(*source.kind(), nebula_resource::ErrorKind::Permanent);
        },
        other => panic!("expected typed terminal failure, got {other:?}"),
    }
    assert_eq!(*failing.destroyed.lock().unwrap(), vec![0]);
    assert_eq!(
        *sibling.destroyed.lock().unwrap(),
        vec![0],
        "one failing row must not discard sibling teardown"
    );
}

#[tokio::test]
async fn resident_lease_release_does_not_destroy_retained_master() {
    let (manager, resource) = resident_lifecycle_manager();
    let first = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    let second = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    assert_eq!(first.0, second.0);
    assert_eq!(first.release().await.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(second.release().await.unwrap(), ReleaseOutcome::Completed);
    assert!(resource.destroyed.lock().unwrap().is_empty());
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(*resource.destroyed.lock().unwrap(), vec![0]);
}

#[tokio::test]
async fn resident_replacement_destroys_old_master_only_after_last_lease() {
    let (manager, resource) = resident_lifecycle_manager();
    let old = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    resource.alive.store(false, Ordering::SeqCst);
    let new = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    assert_eq!(old.0, 0);
    assert_eq!(new.0, 1);
    assert!(resource.destroyed.lock().unwrap().is_empty());
    assert_eq!(old.release().await.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(*resource.destroyed.lock().unwrap(), vec![0]);
    assert_eq!(new.release().await.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(*resource.destroyed.lock().unwrap(), vec![0]);
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(*resource.destroyed.lock().unwrap(), vec![0, 1]);
}

#[tokio::test]
async fn resident_predecessor_teardown_failure_does_not_cancel_healthy_replacement() {
    let (manager, resource) = resident_lifecycle_manager();
    let old = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    assert_eq!(old.0, 0);
    assert_eq!(old.release().await.unwrap(), ReleaseOutcome::Completed);

    resource.alive.store(false, Ordering::SeqCst);
    resource.fail_destroy.store(true, Ordering::SeqCst);
    let replacement = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("predecessor teardown failure must not cancel its healthy replacement");

    assert_eq!(replacement.0, 1);
    assert_eq!(
        *resource.destroyed.lock().unwrap(),
        vec![0],
        "failed predecessor teardown is attempted exactly once"
    );
    resource.fail_destroy.store(false, Ordering::SeqCst);
    assert_eq!(
        replacement.release().await.unwrap(),
        ReleaseOutcome::Completed
    );
    assert_eq!(
        *resource.destroyed.lock().unwrap(),
        vec![0],
        "the usable replacement remains retained after the acquire succeeds"
    );

    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(*resource.destroyed.lock().unwrap(), vec![0, 1]);
}

#[tokio::test]
async fn resident_failed_recreation_preserves_master_for_terminal_teardown() {
    let (manager, resource) = resident_lifecycle_manager();
    let old = manager
        .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    resource.alive.store(false, Ordering::SeqCst);
    resource.fail_create.store(true, Ordering::SeqCst);
    assert!(
        manager
            .acquire_resident::<ResidentLifecycleResource>(&test_ctx(), &AcquireOptions::default())
            .await
            .is_err()
    );
    assert_eq!(old.release().await.unwrap(), ReleaseOutcome::Completed);
    assert!(resource.destroyed.lock().unwrap().is_empty());
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(*resource.destroyed.lock().unwrap(), vec![0]);
}

// ---------------------------------------------------------------------------
// Handle RAII semantics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tainted_handle_not_recycled() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let mut handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .unwrap();

    handle.taint();
    drop(handle);
    // A tainted handle is destroyed, not recycled — wait for the release
    // worker to run `destroy` (idle stays 0 throughout, so the destroy
    // counter is the deterministic completion signal here).
    wait_count_at_least(&resource.destroy_counter, 1).await;

    // Tainted handle should NOT be recycled.
    assert_eq!(idle_count::<PoolTestResource>(&mgr).await, 0);
}

/// Explicitly discarding a lease releases capacity without detaching ownership.
#[tokio::test]
async fn explicit_discard_destroys_instead_of_returning_to_pool() {
    let resource = PoolTestResource::new();
    let manager = Manager::new();
    register_pool(
        &manager,
        resource.clone(),
        test_config(),
        Pooled::new(Default::default(), 1),
    );
    let mut guard = manager
        .acquire_pooled::<PoolTestResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    guard.taint();
    assert_eq!(guard.release().await.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(idle_count::<PoolTestResource>(&manager).await, 0);
    assert_eq!(resource.destroy_counter.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn panic_in_drop_cleanup_does_not_abort_or_leak_drain_reservation() {
    let manager = Manager::new();
    let resource = PanickingDestroyPoolResource::new();
    register_pool(
        &manager,
        resource,
        test_config(),
        Pooled::new(Default::default(), 1),
    );
    let mut guard = manager
        .acquire_pooled::<PanickingDestroyPoolResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    guard.taint();
    drop(guard);
    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert_eq!(
        report.dropped_release_tasks, 1,
        "the panicking teardown was attempted and counted once"
    );
}

#[tokio::test]
async fn release_guarded_handle_runs_teardown_and_returns_ok() {
    let manager = Manager::new();
    let resource = DropOnRecycleResource::new();
    register_pool(
        &manager,
        resource.clone(),
        test_config(),
        Pooled::new(Default::default(), 1),
    );
    let guard = manager
        .acquire_pooled::<DropOnRecycleResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    assert_eq!(guard.release().await.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(
        resource.destroy_counter.load(Ordering::Relaxed),
        1,
        "release does not return before the provider's teardown completes"
    );
}

// budget-justified: slow-destroy test fixture + cancel-safety regression test for the P1 release() finding
/// A pooled resource whose `destroy` is deliberately slow, so a test can
/// cancel an in-flight `release()` while its teardown is still running.
#[derive(Clone)]
struct SlowDestroyPoolResource {
    create_counter: Arc<AtomicU64>,
    destroy_counter: Arc<AtomicU64>,
}

impl SlowDestroyPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for SlowDestroyPoolResource {
    type Config = TestConfig;
    type Instance = ();
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("slow-destroy-pool")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn destroy(&self, _runtime: (), _cx: nebula_resource::TeardownCx) -> Result<(), Error> {
        // Long enough that a 20ms release() timeout reliably fires first.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("slow-destroy-pool"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(SlowDestroyPoolResource);

#[async_trait::async_trait]
impl PoolProvider for SlowDestroyPoolResource {}

#[tokio::test]
async fn release_teardown_survives_caller_cancellation() {
    // P1 regression: if the task awaiting `release()` is cancelled mid-teardown,
    // the teardown must STILL run to completion and the drain must STILL settle
    // (both run on a task detached from the caller's cancellation). Otherwise a
    // pooled runtime leaks un-destroyed and `graceful_shutdown` / revoke wedge
    // on a permanently-counted slot.
    let manager = Manager::new();
    let resource = SlowDestroyPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        ..Default::default()
    };
    let pool_rt = Pooled::<SlowDestroyPoolResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let mut handle: ResourceGuard<SlowDestroyPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    // Taint so release() forces a destroy (the slow teardown path).
    handle.taint();

    // Cancel the release while its teardown is still sleeping: the 300ms destroy
    // cannot finish within the 20ms timeout, so the future awaiting `release()`
    // is dropped mid-teardown.
    let cancelled =
        tokio::time::timeout(std::time::Duration::from_millis(20), handle.release()).await;
    assert!(
        cancelled.is_err(),
        "the slow release() must have been cancelled by the short timeout"
    );

    // The detached teardown task must still complete the destroy...
    let destroyed = poll_until(std::time::Duration::from_secs(3), || {
        resource.destroy_counter.load(Ordering::Relaxed) >= 1
    })
    .await;
    assert!(
        destroyed,
        "teardown must complete on its detached task despite the caller being cancelled"
    );

    // ...and the drain must have settled, so shutdown does not hang.
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        manager.graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        ),
    )
    .await
    .expect("graceful_shutdown must not hang — the cancelled release still settled the drain")
    .expect("graceful_shutdown must succeed");
}

// budget-justified: hang/panic destroy fixtures + release() author-hook-bound regression tests
/// A pooled resource whose `destroy` never completes, modelling a careless
/// author `Provider::destroy` that hangs forever. `release().await` must not
/// wedge on it — the shared `hook_guard` ceiling bounds the teardown.
#[derive(Clone)]
struct HangingDestroyPoolResource {
    create_counter: Arc<AtomicU64>,
}

impl HangingDestroyPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for HangingDestroyPoolResource {
    type Config = TestConfig;
    type Instance = ();
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("hanging-destroy-pool")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn destroy(&self, _runtime: (), _cx: nebula_resource::TeardownCx) -> Result<(), Error> {
        // Hangs forever: the only thing that may unwedge `release()` is the
        // shared author-hook ceiling, not this future ever resolving.
        std::future::pending::<()>().await;
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("hanging-destroy-pool"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(HangingDestroyPoolResource);

#[async_trait::async_trait]
impl PoolProvider for HangingDestroyPoolResource {}

/// A pooled resource whose `destroy` panics, modelling a careless author
/// `Provider::destroy` that unwinds. `release().await` must surface a typed
/// error (panic isolated) rather than crash the caller.
#[derive(Clone)]
struct PanickingDestroyPoolResource {
    create_counter: Arc<AtomicU64>,
}

impl PanickingDestroyPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for PanickingDestroyPoolResource {
    type Config = TestConfig;
    type Instance = ();
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("panicking-destroy-pool")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn destroy(&self, _runtime: (), _cx: nebula_resource::TeardownCx) -> Result<(), Error> {
        panic!("author Provider::destroy panics on purpose");
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("panicking-destroy-pool"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(PanickingDestroyPoolResource);

#[async_trait::async_trait]
impl PoolProvider for PanickingDestroyPoolResource {}

#[tokio::test(start_paused = true)]
async fn release_bounds_a_hanging_author_teardown() {
    // A careless author `Provider::destroy` that hangs forever must NOT wedge
    // a caller that awaited `release()`. Per ADR-0093 the per-resource teardown
    // deadline is the effective bound: a tainted lease is a revoke teardown, so
    // the default 30s budget is capped to the 5s revoke cap, and `destroy_within`
    // abandons the hang with a typed `Cancelled` error (a teardown abandonment,
    // NOT a retryable `Backpressure` overload) well before the outer
    // `hook_guard` ceiling. `start_paused` advances virtual time to the deadline
    // instantly + deterministically — no wall-clock wait.
    let manager = Manager::new();
    let resource = HangingDestroyPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        ..Default::default()
    };
    let pool_rt = Pooled::<HangingDestroyPoolResource>::new(pool_config, 1);
    register_pool(&manager, resource.clone(), test_config(), pool_rt);

    let ctx = test_ctx();
    let mut handle: ResourceGuard<HangingDestroyPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    // Taint so release() forces the (hanging) destroy path, not a recycle.
    handle.taint();

    let outcome = handle.release().await;
    let err = outcome.expect_err("a hanging author teardown must make release() return Err");
    assert!(
        err.to_string().contains("exceeded teardown budget"),
        "release() must surface the per-resource teardown-budget bound, got: {err}"
    );
    assert!(
        !err.is_retryable(),
        "an abandoned teardown is a Cancelled-class error, not a retryable backpressure, got: {err}"
    );
}

#[tokio::test]
async fn release_isolates_a_panicking_author_teardown() {
    // A careless author `Provider::destroy` that panics must NOT crash the
    // caller that awaited `release()`: the teardown runs through the shared
    // `hook_guard` chokepoint, which catches the unwind and surfaces a typed
    // error. Reaching the assertion at all proves the process was not aborted.
    let manager = Manager::new();
    let resource = PanickingDestroyPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 0,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        ..Default::default()
    };
    let pool_rt = Pooled::<PanickingDestroyPoolResource>::new(pool_config, 1);
    register_pool(&manager, resource.clone(), test_config(), pool_rt);

    let ctx = test_ctx();
    let mut handle: ResourceGuard<PanickingDestroyPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    // Taint so release() forces the (panicking) destroy path, not a recycle.
    handle.taint();

    let outcome = handle.release().await;
    let err = outcome.expect_err("a panicking author teardown must make release() return Err");
    assert!(
        err.to_string().contains("panicked"),
        "release() must surface the isolated-panic message, got: {err}"
    );
    // Permanent, not transient: a teardown panic is an author-hook bug (a broken `destroy`/
    // `on_release` impl), not a condition that resolves with time or
    // backoff — retrying the SAME instance's teardown would panic again
    // deterministically. Must classify Permanent (not retryable), matching
    // every other `HookFault::Panicked` site in the crate.
    assert!(
        !err.is_retryable(),
        "a teardown panic must classify as a permanent author-hook bug, \
         not a retryable condition, got: {err:?}"
    );
}

// budget-justified: panicking-create warmup fixture + warmup author-hook-bound regression test
/// A pooled resource whose `create` panics, modelling a careless author
/// `Provider::create` that unwinds during warmup. `warmup_pool` runs
/// `create_entry` (→ `Provider::create`) through the shared `hook_guard`
/// chokepoint, so the panic is caught + isolated and `warmup_pool` returns a
/// typed Permanent error rather than crashing the caller.
#[derive(Clone)]
struct PanickingCreatePoolResource;

#[async_trait::async_trait]
impl Provider for PanickingCreatePoolResource {
    type Config = TestConfig;
    type Instance = ();
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("panicking-create-pool")
    }

    async fn create(&self, _config: &TestConfig, _ctx: &ResourceContext) -> Result<(), Error> {
        panic!("author Provider::create panics on purpose during warmup");
    }

    async fn destroy(&self, _runtime: (), _cx: nebula_resource::TeardownCx) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("panicking-create-pool"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(PanickingCreatePoolResource);

#[async_trait::async_trait]
impl PoolProvider for PanickingCreatePoolResource {}

#[tokio::test]
async fn warmup_isolates_a_panicking_author_create() {
    // A careless author `Provider::create` that panics during warmup must NOT
    // crash the caller: `warmup_pool` runs `create_entry` through the shared
    // `hook_guard` chokepoint, which catches the unwind and surfaces a typed
    // Permanent error. `min_size: 1` makes warmup target exactly one create,
    // so the panic fires. Reaching the assertion at all proves the process was
    // not aborted.
    let manager = Manager::new();
    let resource = PanickingCreatePoolResource;
    let pool_config = nebula_resource::topology::pooled::config::Config {
        min_size: 1,
        max_size: 4,
        idle_timeout: None,
        max_lifetime: None,
        ..Default::default()
    };
    let pool_rt = Pooled::<PanickingCreatePoolResource>::new(pool_config, 1);
    register_pool(&manager, resource, test_config(), pool_rt);

    let ctx = test_ctx();
    let err = manager
        .warmup_pool::<PanickingCreatePoolResource>(&ctx)
        .await
        .expect_err("a panicking author create must make warmup_pool return Err");
    assert!(
        err.to_string().contains("panicked"),
        "warmup_pool must surface the isolated-panic message, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// 2. Pool stale fingerprint evicts idle entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_stale_fingerprint_evicts_idle_entry() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire + release to populate idle.
    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;
    assert_eq!(idle_count::<PoolTestResource>(&mgr).await, 1);

    // Change the config fingerprint via reload — makes the idle entry stale.
    // `reload_config` bumps the pool fingerprint through the framework, so the
    // idle slot is rejected by `accept` on the next acquire.
    mgr.reload_config::<PoolTestResource>(
        TestConfig {
            name: "stale-evict-v2".into(),
        },
        &ScopeLevel::Global,
    )
    .expect("reload bumps fingerprint");

    // Next acquire should destroy stale entry and create fresh.
    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed after fingerprint change");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "stale fingerprint should have forced a fresh creation"
    );

    drop(handle2);
}

// ---------------------------------------------------------------------------
// 3. Pool max_lifetime evicts expired entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pool_max_lifetime_evicts_expired_entry() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        max_lifetime: Some(std::time::Duration::from_millis(50)),
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire + release to populate idle.
    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    drop(handle);
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;
    assert_eq!(idle_count::<PoolTestResource>(&mgr).await, 1);

    // Sleep past max_lifetime — a deliberate clock advance (the entry must
    // actually age beyond its lifetime), not a release-settle guess.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Next acquire should destroy expired entry and create fresh.
    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed after max_lifetime expiry");

    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "expired entry should have forced a fresh creation"
    );

    drop(handle2);
}

// ---------------------------------------------------------------------------
// 4. Pool recycle Drop decision destroys entry
// ---------------------------------------------------------------------------

/// A pool resource whose `recycle()` always returns `RecycleDecision::Drop`.
#[derive(Clone)]
struct DropOnRecycleResource {
    create_counter: Arc<AtomicU64>,
    /// Release-completion signal: the idle count stays `0` for this resource
    /// (every release ends in `destroy`), so the destroy counter — not the
    /// idle count — is the deterministic "release ran" signal.
    destroy_counter: Arc<AtomicU64>,
}

impl DropOnRecycleResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for DropOnRecycleResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("drop-on-recycle")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("drop-on-recycle"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(DropOnRecycleResource);

impl PoolProvider for DropOnRecycleResource {
    async fn recycle(
        &self,
        _instance: &Arc<AtomicU64>,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Drop)
    }
}

#[tokio::test]
async fn pool_recycle_drop_destroys_entry() {
    let resource = DropOnRecycleResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<DropOnRecycleResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Acquire + release. Entry should NOT return to idle because recycle
    // returns Drop.
    let handle = mgr
        .acquire_pooled::<DropOnRecycleResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    drop(handle);
    // recycle=Drop destroys the instance — wait for `destroy` to run (idle
    // stays 0 throughout, so the destroy counter is the settle signal).
    assert!(
        poll_until(std::time::Duration::from_secs(2), || {
            resource.destroy_counter.load(Ordering::Relaxed) >= 1
        })
        .await,
        "destroy never ran for recycle=Drop"
    );

    assert_eq!(
        idle_count::<DropOnRecycleResource>(&mgr).await,
        0,
        "recycle=Drop should not return entry to idle"
    );
}

// ---------------------------------------------------------------------------
// ADR-0093: credentialed pooled resources DISCARD on the default `recycle`
// ---------------------------------------------------------------------------

/// A credentialed pooled resource (declares a `#[credential]` slot at the type
/// level) that leaves `PoolProvider::recycle` at its default. Under ADR-0093
/// the default is safe-by-construction: a credentialed pooled instance is
/// session-stateful, so the framework DISCARDS it on release rather than
/// re-pool a dirty instance.
#[derive(Clone)]
struct CredentialedDefaultPoolResource {
    create_counter: Arc<AtomicU64>,
    destroy_counter: Arc<AtomicU64>,
}

impl CredentialedDefaultPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for CredentialedDefaultPoolResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("cred-pool-default")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("cred-pool-default"),
            "",
        )
    }
}

impl HasCredentialSlots for CredentialedDefaultPoolResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    // Hand-mirrors what the derive emits for a `#[credential]`-bearing struct:
    // this is the combination ADR-0093 safe default targets (Pooled +
    // credentialed). `recycle` is intentionally left at its default below.
    fn declares_credential_slots() -> bool {
        true
    }
}

// Default `recycle` — under ADR-0093 a credentialed pooled resource DISCARDS.
impl PoolProvider for CredentialedDefaultPoolResource {}

/// A credentialed pooled resource that OVERRIDES `recycle` to return `Keep`,
/// modeling an author who wipes per-lease session state and so opts back into
/// pooling. This re-enables instance reuse despite the credential slot.
#[derive(Clone)]
struct CredentialedKeepPoolResource {
    create_counter: Arc<AtomicU64>,
}

impl CredentialedKeepPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for CredentialedKeepPoolResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("cred-pool-keep")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("cred-pool-keep"),
            "",
        )
    }
}

impl HasCredentialSlots for CredentialedKeepPoolResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }
}

impl PoolProvider for CredentialedKeepPoolResource {
    async fn recycle(
        &self,
        _instance: &Arc<AtomicU64>,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, Error> {
        // Author wipes per-lease session state, so pooling is safe to re-enable.
        Ok(RecycleDecision::Keep)
    }
}

/// ADR-0093 safe default: a credentialed pooled resource on the DEFAULT
/// `recycle` DISCARDS its instance on a clean release — the instance is never
/// returned to idle and a subsequent acquire creates a fresh one (no
/// cross-lease state bleed). Asserted three ways: the destroy counter fires,
/// idle stays empty, and the metrics snapshot records `discarded` not
/// `recycled`.
#[tokio::test]
async fn credentialed_pool_default_recycle_discards() {
    let resource = CredentialedDefaultPoolResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<CredentialedDefaultPoolResource>::new(config, 1);
    let mgr = pool_manager_with_metrics(resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let handle = mgr
        .acquire_pooled::<CredentialedDefaultPoolResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);

    // Discarded-not-recycled: idle stays 0, so the destroy counter is the
    // settle signal (the same event the discarded counter observes).
    wait_count_at_least(&resource.destroy_counter, 1).await;
    assert_eq!(
        idle_count::<CredentialedDefaultPoolResource>(&mgr).await,
        0,
        "credentialed default recycle must DISCARD, not return to idle"
    );

    let snap = mgr
        .metrics()
        .expect("manager was built with a metrics registry")
        .snapshot()
        .recycle_outcomes;
    assert_eq!(
        snap.discarded, 1,
        "credentialed default recycle must record discarded"
    );
    assert_eq!(
        snap.recycled, 0,
        "credentialed default recycle must not recycle"
    );

    // A subsequent acquire creates a fresh instance — nothing was reused.
    let handle2 = mgr
        .acquire_pooled::<CredentialedDefaultPoolResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "discarded instance must not be reused — a fresh one is created"
    );
    drop(handle2);
}

/// ADR-0093 control: a non-credentialed pooled resource still RECYCLES on the
/// default `recycle` (Keep) — the safe default only changes behavior for
/// credentialed resources. The instance returns to idle and is reused.
#[tokio::test]
async fn non_credentialed_pool_default_recycle_keeps() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);

    // Default Keep for a slot-less resource: instance returns to idle.
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;

    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        1,
        "non-credentialed default recycle KEEPS — the idle instance is reused"
    );
    drop(handle2);
}

/// ADR-0093 opt-in: a credentialed pooled resource that OVERRIDES `recycle` to
/// return `Keep` (modeling an author who wipes session state) re-enables
/// pooling — the instance returns to idle and a subsequent acquire reuses it.
#[tokio::test]
async fn credentialed_pool_recycle_keep_override_reuses() {
    let resource = CredentialedKeepPoolResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<CredentialedKeepPoolResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let handle = mgr
        .acquire_pooled::<CredentialedKeepPoolResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);

    // The Keep override returns the instance to idle despite the credential slot.
    wait_idle_count::<CredentialedKeepPoolResource>(&mgr, 1).await;

    let handle2 = mgr
        .acquire_pooled::<CredentialedKeepPoolResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        1,
        "credentialed `recycle -> Keep` override re-enables reuse"
    );
    drop(handle2);
}

// ---------------------------------------------------------------------------
// ResourceGuard::release() — explicit, awaited release checkpoint (canon §11.4)
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn graceful_shutdown_executes_guard_dropped_after_signal() {
    let resource = PoolTestResource::new();
    let manager = Manager::new();
    let pool = Pooled::<PoolTestResource>::new(Default::default(), 1);
    register_pool(&manager, resource.clone(), test_config(), pool);
    let ctx = test_ctx();
    let mut guard = manager
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire succeeds");
    guard.taint();
    let shutdown = manager.graceful_shutdown(ShutdownConfig::default());
    tokio::pin!(shutdown);
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    assert!(manager.is_shutdown());
    // Run idle workers after SIGNAL before submitting the late teardown.
    tokio::task::yield_now().await;
    drop(guard);
    let report = shutdown.await.expect("shutdown drains late teardown");
    assert!(report.release_queue_drained);
    assert_eq!(resource.destroy_counter.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn graceful_shutdown_accepts_maximum_release_queue_budget() {
    let manager = Manager::new();
    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default().with_release_queue_timeout(std::time::Duration::MAX),
        )
        .await
        .expect("a representable duration must not panic during shutdown");
    assert!(report.registry_cleared);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test]
async fn graceful_shutdown_destroys_idle_pool_instances() {
    let resource = PoolTestResource::new();
    let manager = Manager::new();
    register_pool(
        &manager,
        resource.clone(),
        test_config(),
        Pooled::new(Default::default(), 1),
    );
    let ctx = test_ctx();
    let guard = manager
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire succeeds");
    let outcome = guard
        .release()
        .await
        .expect("healthy entry returns to idle");
    assert_eq!(outcome, ReleaseOutcome::Completed);
    assert_eq!(idle_count::<PoolTestResource>(&manager).await, 1);
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("terminal cleanup succeeds");
    assert_eq!(
        resource.destroy_counter.load(Ordering::Relaxed),
        1,
        "terminal shutdown destroys idle entries instead of dropping only the registry row"
    );
}

#[tokio::test(start_paused = true)]
async fn aborted_or_cancelled_drain_keeps_late_cleanup_available() {
    for cancel_drain in [false, true] {
        let resource = PoolTestResource::new();
        let manager = Manager::new();
        register_pool(
            &manager,
            resource.clone(),
            test_config(),
            Pooled::new(Default::default(), 1),
        );
        let ctx = test_ctx();
        let mut guard = manager
            .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire succeeds");
        guard.taint();
        {
            let shutdown = manager.graceful_shutdown(
                ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_secs(1)),
            );
            tokio::pin!(shutdown);
            assert!(futures::poll!(shutdown.as_mut()).is_pending());
            if !cancel_drain {
                let error = shutdown
                    .await
                    .expect_err("outstanding guard prevents drain");
                assert!(matches!(
                    error,
                    nebula_resource::ShutdownError::DrainTimeout { outstanding: 1 }
                ));
            }
        }
        assert!(manager.contains(&PoolTestResource::key()));
        tokio::task::yield_now().await;
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while resource.destroy_counter.load(Ordering::Relaxed) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("late teardown remains available");
        assert_eq!(resource.destroy_counter.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn forced_shutdown_reports_outstanding_guard_without_claiming_cleanup() {
    let resource = PoolTestResource::new();
    let manager = Manager::new();
    register_pool(
        &manager,
        resource.clone(),
        test_config(),
        Pooled::new(Default::default(), 1),
    );
    let ctx = test_ctx();
    let mut guard = manager
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire succeeds");
    guard.taint();
    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(std::time::Duration::from_secs(1))
                .with_drain_timeout_policy(nebula_resource::DrainTimeoutPolicy::Force),
        )
        .await
        .expect("force proceeds past outstanding guard");
    assert_eq!(report.outstanding_handles_after_drain, 1);
    assert_eq!(
        report.dropped_release_tasks, 0,
        "snapshot precedes late submission"
    );
    assert!(
        !report.release_queue_drained,
        "Force retains live cleanup workers"
    );
    let outcome = guard
        .release()
        .await
        .expect("late release is still accepted while manager lives");
    assert_eq!(outcome, ReleaseOutcome::Completed);
    assert_eq!(resource.destroy_counter.load(Ordering::Relaxed), 1);
    assert_eq!(
        report.outstanding_handles_after_drain, 1,
        "the report remains a snapshot"
    );
}

/// `release()` on a Pooled guard returns `Completed` and recycles the instance:
/// the slot lands back in idle and a subsequent acquire reuses it (the
/// `create_counter` does not advance). This is the awaited-inline counterpart
/// to the drop-then-`wait_idle_count` recycle path.
#[tokio::test]
async fn release_pooled_guard_recycles_and_returns_ok() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);

    // Explicit awaited release: runs the recycle on the detached teardown task
    // and awaits its completion, so the slot is back in idle when it returns.
    let outcome = handle
        .release()
        .await
        .expect("release of a healthy pooled guard recycles and returns Ok");
    assert_eq!(outcome, ReleaseOutcome::Completed);

    // The instance is back in idle by the time `release()` returned (the
    // teardown task ran the recycle to completion before `release().await`
    // resolved) — no settle needed.
    assert_eq!(
        idle_count::<PoolTestResource>(&mgr).await,
        1,
        "an awaited release must have recycled the instance back to idle"
    );

    // Reacquire reuses the recycled instance: no new creation.
    let handle2 = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        1,
        "release() recycled, not destroyed — reacquire reuses the instance"
    );

    drop(handle2);
}

/// `release()` on an Owned (resident) guard returns `Ok(())` — there is no
/// recycle/destroy work, only the drain + event settle.
#[tokio::test]
async fn release_owned_resident_guard_returns_ok() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    let outcome = handle
        .release()
        .await
        .expect("release of an owned resident guard is a no-op teardown — Ok");
    assert_eq!(outcome, ReleaseOutcome::Completed);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

/// Calling `release()` then dropping the (consumed) guard must emit **exactly
/// one** `Released` event and decrement the drain counters exactly once: the
/// `release()` consumes `self`, so the subsequent drop of the husk is inert.
#[tokio::test]
async fn release_then_drop_emits_exactly_one_released_event() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let mut rx = manager.subscribe_events();

    let ctx = test_ctx();
    let handle: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire should succeed");

    // The awaited checkpoint runs the settle (emits `Released`) and consumes
    // `self`; the husk's drop at end of statement is fully inert.
    assert_eq!(
        handle.release().await.expect("release should succeed"),
        ReleaseOutcome::Completed
    );

    let mut released_count = 0usize;
    while let Some(event) = rx.try_recv() {
        if matches!(
            &event,
            nebula_resource::ResourceEvent::Released { key, .. }
                if key == &resource_key!("test-resident")
        ) {
            released_count += 1;
        }
    }
    assert_eq!(
        released_count, 1,
        "release() then drop must emit EXACTLY one Released event — the \
         drop-after-release husk is inert (no double emit / double decrement)"
    );

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

/// Queue rejection settles the lease reservation but must not claim that
/// provider cleanup completed.
#[tokio::test]
async fn rejected_release_never_emits_released() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("registration should succeed");

    let mut events = manager.subscribe_events();
    let guard: ResourceGuard<ResidentTestResource> = manager
        .acquire_resident(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("acquire should succeed");
    drop(manager);

    guard
        .release()
        .await
        .expect_err("closed cleanup queue rejects release submission");
    while let Some(event) = events.try_recv() {
        assert!(
            !matches!(event, nebula_resource::ResourceEvent::Released { .. }),
            "a rejected cleanup must never be observed as Released"
        );
    }
}

// ---------------------------------------------------------------------------
// Fail-closed slot-name validation on a no-slot resource
// ---------------------------------------------------------------------------

/// A `no_credential_slots!` resource (`declares_credential_slots() == false`,
/// `credential_slot_names() == &[]`) has no slot to rotate, so
/// `refresh_slot`/`taint_slot` must reject EVERY slot name — not just names
/// that happen to collide with some other resource's declared slots. Before
/// the fail-closed fix, `accepts_credential_slot_name` short-circuited to
/// `true` whenever `declares_credential_slots()` was `false`, so a typo'd
/// slot name on a no-slot resource silently reached `taint_slot`'s
/// destructive taint + revoke-epoch bump instead of being rejected.
#[tokio::test]
async fn refresh_and_taint_slot_reject_any_name_on_a_no_slot_resource() {
    let resource = PoolTestResource::new();
    let config = nebula_resource::topology::pooled::config::Config {
        max_size: 2,
        ..Default::default()
    };
    let pool = Pooled::<PoolTestResource>::new(config, 1);
    let mgr = Manager::new();
    register_pool(&mgr, resource.clone(), test_config(), pool);
    let ctx = test_ctx();

    // Seed one idle entry so "the pool still serves" is an observable fact,
    // not just an absence of a panic.
    let handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("initial acquire must succeed");
    drop(handle);
    wait_idle_count::<PoolTestResource>(&mgr, 1).await;

    let key = resource_key!("test-pool");

    let refresh_err = mgr
        .refresh_slot(&key, ScopeLevel::Global, "totally-made-up")
        .await
        .expect_err("a no-slot resource must reject every slot name");
    assert!(
        refresh_err.to_string().contains("unknown credential slot"),
        "expected an unknown-credential-slot rejection, got: {refresh_err}"
    );

    let taint_err = mgr
        .taint_slot(&key, ScopeLevel::Global, "totally-made-up")
        .expect_err("taint_slot must also reject every slot name on a no-slot resource");
    assert!(
        taint_err.to_string().contains("unknown credential slot"),
        "expected an unknown-credential-slot rejection, got: {taint_err}"
    );

    // No taint happened: the pool's idle entry must still be there, and a
    // fresh acquire must still succeed — the rejected slot-name calls left
    // the resource completely untouched.
    assert_eq!(
        idle_count::<PoolTestResource>(&mgr).await,
        1,
        "a rejected slot-name call must not taint or otherwise disturb the pool"
    );
    let final_handle = mgr
        .acquire_pooled::<PoolTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("the pool must still serve acquires after the rejected slot-name calls");
    drop(final_handle);
}
