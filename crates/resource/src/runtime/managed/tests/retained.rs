use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use arc_swap::ArcSwap;
use nebula_core::{ResourceKey, resource_key};
use tokio::sync::Notify;

use crate::{
    Error, ErrorKind, Provider, ReplaceStatus, ResourceContext, ResourceMetadata, ResourceStatus,
    RetainStatus, RetainedId, RetainedStore, TeardownCx,
    release_queue::{ReleaseQueue, ReleaseQueueHandle},
    resource::ResourceConfig,
    runtime::managed::ManagedResource,
    topology::{CreatedEntry, Ticket, Topology, Unavailable, store::InstanceStore},
};

#[derive(Clone)]
struct RetainedConfig;

crate::impl_empty_has_schema!(RetainedConfig);

impl ResourceConfig for RetainedConfig {
    fn fingerprint(&self) -> u64 {
        0
    }
}

struct Root {
    serial: u64,
    raw_drops: Arc<AtomicUsize>,
}

impl Drop for Root {
    fn drop(&mut self) {
        self.raw_drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
enum QuiesceBehavior {
    ConstructionPanic,
    Complete,
    Panic,
    Hang,
}

struct RetainedTopology {
    behavior: QuiesceBehavior,
    retained_id: Mutex<Option<RetainedId>>,
    hook_entered: Arc<Notify>,
    release_hook: Arc<Notify>,
    quiesce_entered: Arc<Notify>,
}

impl RetainedTopology {
    fn new(behavior: QuiesceBehavior) -> Self {
        Self {
            behavior,
            retained_id: Mutex::new(None),
            hook_entered: Arc::new(Notify::new()),
            release_hook: Arc::new(Notify::new()),
            quiesce_entered: Arc::new(Notify::new()),
        }
    }

    fn set_retained_id(&self, id: RetainedId) {
        *self
            .retained_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(id);
    }
}

#[derive(Clone)]
struct RetainedResource {
    destroyed: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for RetainedResource {
    type Config = RetainedConfig;
    type Instance = Root;
    type Topology = RetainedTopology;

    fn key() -> ResourceKey {
        resource_key!("managed-retained-regression")
    }

    async fn create(
        &self,
        _config: &RetainedConfig,
        _ctx: &ResourceContext,
    ) -> Result<Root, Error> {
        Err(Error::permanent("test creates retained roots explicitly"))
    }

    async fn destroy(&self, runtime: Root, _context: TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        drop(runtime);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

crate::no_credential_slots!(RetainedResource);

impl Topology<RetainedResource> for RetainedTopology {
    type Entry = Arc<Root>;

    fn try_reserve(&self, _store: &InstanceStore<Self::Entry>) -> Result<Ticket, Unavailable> {
        Ok(Ticket::infallible())
    }

    async fn create_entry(
        &self,
        _resource: &RetainedResource,
        _config: &RetainedConfig,
        _context: &ResourceContext,
        _retained: &RetainedStore<Self::Entry>,
    ) -> Result<CreatedEntry<Self::Entry>, Error> {
        Err(Error::permanent("test creates retained roots explicitly"))
    }

    fn entry_instance<'entry>(&self, entry: &'entry Self::Entry) -> &'entry Root {
        entry
    }

    fn into_owned_instance(&self, entry: Self::Entry) -> Option<Root> {
        Arc::into_inner(entry)
    }

    fn quiesce(&self) -> impl Future<Output = Result<(), Error>> + Send {
        if matches!(self.behavior, QuiesceBehavior::ConstructionPanic) {
            panic!("intentional quiesce future construction panic");
        }
        async move {
            self.quiesce_entered.notify_one();
            match self.behavior {
                QuiesceBehavior::Complete => Ok(()),
                QuiesceBehavior::Panic | QuiesceBehavior::ConstructionPanic => {
                    panic!("intentional topology quiesce panic")
                },
                QuiesceBehavior::Hang => std::future::pending().await,
            }
        }
    }

    async fn dispatch_credential_hook(
        &self,
        _resource: &RetainedResource,
        _store: &InstanceStore<Self::Entry>,
        retained: &RetainedStore<Self::Entry>,
        _slot: &str,
        _refresh: bool,
    ) -> Result<(), Error> {
        let retained_id = self
            .retained_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ok_or_else(|| Error::permanent("test retained id was not installed"))?;
        let lease = retained
            .lease(retained_id)
            .ok_or_else(|| Error::cancelled().with_resource_key(RetainedResource::key()))?;
        self.hook_entered.notify_one();
        self.release_hook.notified().await;
        assert_eq!(lease.serial, 1, "the hook kept the original retained root");
        Ok(())
    }
}

fn managed(
    behavior: QuiesceBehavior,
) -> (Arc<ManagedResource<RetainedResource>>, ReleaseQueueHandle) {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let resource = RetainedResource { destroyed };
    let topology = RetainedTopology::new(behavior);
    let (release_queue, workers) = ReleaseQueue::new(1);
    let managed = Arc::new(ManagedResource {
        resource,
        config: ArcSwap::from_pointee(RetainedConfig),
        topology,
        store: InstanceStore::with_abandonment_tracker(None, release_queue.abandonment_tracker()),
        retained: RetainedStore::new(release_queue.abandonment_tracker()),
        release_queue: Arc::new(release_queue),
        generation: AtomicU64::new(0),
        status: ArcSwap::from_pointee(ResourceStatus::new()),
        recovery_gate: None,
        tainted: AtomicBool::new(false),
        in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
        maintenance_sweeps: AtomicU64::new(0),
        maintenance: Default::default(),
    });
    (managed, workers)
}

fn install_root(
    managed: &ManagedResource<RetainedResource>,
    serial: u64,
    raw_drops: Arc<AtomicUsize>,
) -> RetainedId {
    let status = managed
        .retained
        .retain(Arc::new(Root { serial, raw_drops }));
    let RetainStatus::Published(id) = status else {
        panic!("an open retained store must publish the test root");
    };
    managed.topology.set_retained_id(id);
    id
}

async fn stop_queue(managed: &ManagedResource<RetainedResource>, workers: ReleaseQueueHandle) {
    managed.release_queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test]
async fn quiesce_construction_panic_does_not_skip_retained_root_destruction() {
    let (managed, workers) = managed(QuiesceBehavior::ConstructionPanic);
    let raw_drops = Arc::new(AtomicUsize::new(0));
    install_root(&managed, 1, Arc::clone(&raw_drops));
    let outcome = managed.close_retained().await;
    assert_eq!(
        outcome.expect_err("construction panic must surface").kind(),
        &ErrorKind::Permanent
    );
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 1);
    assert_eq!(managed.release_queue.dropped_count(), 0);
    stop_queue(&managed, workers).await;
}

#[tokio::test]
async fn quiesce_panic_does_not_skip_retained_root_destruction() {
    let (managed, workers) = managed(QuiesceBehavior::Panic);
    let raw_drops = Arc::new(AtomicUsize::new(0));
    install_root(&managed, 1, Arc::clone(&raw_drops));

    let outcome = managed.close_retained().await;

    assert_eq!(
        outcome.expect_err("quiesce panic must surface").kind(),
        &ErrorKind::Permanent
    );
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 1);
    assert_eq!(managed.release_queue.dropped_count(), 0);
    stop_queue(&managed, workers).await;
}

#[tokio::test(start_paused = true)]
async fn quiesce_timeout_does_not_skip_retained_root_destruction() {
    let (managed, workers) = managed(QuiesceBehavior::Hang);
    let raw_drops = Arc::new(AtomicUsize::new(0));
    install_root(&managed, 1, Arc::clone(&raw_drops));

    let outcome = managed.close_retained().await;

    assert_eq!(
        outcome.expect_err("quiesce timeout must surface").kind(),
        &ErrorKind::Cancelled
    );
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 1);
    assert_eq!(managed.release_queue.dropped_count(), 0);
    stop_queue(&managed, workers).await;
}

#[tokio::test]
async fn credential_hook_lease_blocks_terminal_drain_across_replacement() {
    let (managed, workers) = managed(QuiesceBehavior::Complete);
    let raw_drops = Arc::new(AtomicUsize::new(0));
    let retained_id = install_root(&managed, 1, Arc::clone(&raw_drops));

    let dispatch_managed = Arc::clone(&managed);
    let dispatch = tokio::spawn(async move {
        dispatch_managed
            .dispatch_slot_hook("credential", true)
            .await
    });
    managed.topology.hook_entered.notified().await;
    assert_eq!(
        managed.retained.replace(
            retained_id,
            Arc::new(Root {
                serial: 2,
                raw_drops: Arc::clone(&raw_drops),
            }),
        ),
        ReplaceStatus::Replaced
    );

    let close_managed = Arc::clone(&managed);
    let close = tokio::spawn(async move { close_managed.close_retained().await });
    managed.topology.quiesce_entered.notified().await;
    tokio::task::yield_now().await;
    assert!(
        !close.is_finished(),
        "terminal cleanup must remain blocked by the credential-hook lease"
    );

    managed.topology.release_hook.notify_one();
    dispatch
        .await
        .expect("dispatch task must not panic")
        .expect("credential hook must complete");
    close
        .await
        .expect("close task must not panic")
        .expect("terminal cleanup must complete");

    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 2);
    assert_eq!(managed.release_queue.dropped_count(), 0);
    stop_queue(&managed, workers).await;
}
