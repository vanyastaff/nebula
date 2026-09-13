use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use arc_swap::ArcSwap;
use nebula_core::{ExecutionId, ResourceKey, resource_key, scope::Scope};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    AcquireOptions, Error, ErrorKind, Provider, ReplaceStatus, ResourceContext,
    ResourceMetadataDraft, ResourceStatus, RetainStatus, RetainedId, RetainedStore, TeardownCx,
    release_queue::{ReleaseQueue, ReleaseQueueHandle, SubmissionOutcome},
    resource::ResourceConfig,
    runtime::{
        acquire_loop::{SlotHookSettlement, SlotHookWaitOutcome},
        managed::ManagedResource,
    },
    topology::{CreatedEntry, Ticket, Topology, Unavailable, store::InstanceStore},
};

#[derive(Clone, nebula_schema::Schema)]
struct RetainedConfig;

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
    hook_completed: Arc<AtomicUsize>,
    release_hook: Arc<Notify>,
    quiesce_entered: Arc<Notify>,
}

impl RetainedTopology {
    fn new(behavior: QuiesceBehavior) -> Self {
        Self {
            behavior,
            retained_id: Mutex::new(None),
            hook_entered: Arc::new(Notify::new()),
            hook_completed: Arc::new(AtomicUsize::new(0)),
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
    destroy_completed: Arc<Notify>,
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
        self.destroy_completed.notify_waiters();
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("managed-retained-regression"),
            "",
        )
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
    ) -> Result<(), crate::topology::HookFault> {
        let retained_id = self
            .retained_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ok_or_else(|| Error::permanent("test retained id was not installed"))
            .map_err(crate::topology::HookFault::Failed)?;
        let lease = retained
            .lease(retained_id)
            .ok_or_else(|| Error::cancelled().with_resource_key(RetainedResource::key()))
            .map_err(crate::topology::HookFault::Failed)?;
        self.hook_entered.notify_one();
        self.release_hook.notified().await;
        assert_eq!(lease.serial, 1, "the hook kept the original retained root");
        self.hook_completed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn managed(
    behavior: QuiesceBehavior,
) -> (Arc<ManagedResource<RetainedResource>>, ReleaseQueueHandle) {
    managed_with_workers(behavior, 1)
}

fn managed_with_workers(
    behavior: QuiesceBehavior,
    worker_count: usize,
) -> (Arc<ManagedResource<RetainedResource>>, ReleaseQueueHandle) {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let resource = RetainedResource {
        destroyed,
        destroy_completed: Arc::new(Notify::new()),
    };
    let topology = RetainedTopology::new(behavior);
    let (release_queue, workers) = ReleaseQueue::new(worker_count);
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

async fn dispatch_test_slot_hook(
    managed: &Arc<ManagedResource<RetainedResource>>,
) -> Result<SubmissionOutcome, Error> {
    let (settlement, admission) = SlotHookSettlement::new(Box::new(|_| {}));
    let accepted = managed.submit_slot_hook(
        "credential",
        true,
        crate::hook_guard::MAX_ROTATION_DISPATCH_CEILING,
        settlement,
        admission,
    )?;
    match accepted
        .wait_until(tokio::time::Instant::now() + std::time::Duration::from_mins(1))
        .await
    {
        SlotHookWaitOutcome::Completed => Ok(SubmissionOutcome::Completed),
        SlotHookWaitOutcome::Deferred(_) => Ok(SubmissionOutcome::Deferred),
        SlotHookWaitOutcome::Abandoned => Err(Error::cancelled()),
        SlotHookWaitOutcome::Failed(error) | SlotHookWaitOutcome::TimedOut(error) => Err(error),
    }
}

fn test_context() -> ResourceContext {
    ResourceContext::minimal(
        Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        },
        CancellationToken::new(),
    )
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
    let dispatch = tokio::spawn(async move { dispatch_test_slot_hook(&dispatch_managed).await });
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
    let dispatch_outcome = dispatch
        .await
        .expect("dispatch task must not panic")
        .expect("credential hook must complete");
    assert_eq!(dispatch_outcome, SubmissionOutcome::Completed);
    close
        .await
        .expect("close task must not panic")
        .expect("terminal cleanup must complete");

    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 2);
    assert_eq!(managed.release_queue.dropped_count(), 0);
    stop_queue(&managed, workers).await;
}

#[tokio::test(start_paused = true)]
async fn last_lease_drop_resumes_blocked_retirement_without_external_poke() {
    let (managed, workers) = managed_with_workers(QuiesceBehavior::Complete, 2);
    let raw_drops = Arc::new(AtomicUsize::new(0));
    let leased_id = install_root(&managed, 1, Arc::clone(&raw_drops));
    let RetainStatus::Published(ready_id) = managed.retained.retain(Arc::new(Root {
        serial: 2,
        raw_drops: Arc::clone(&raw_drops),
    })) else {
        panic!("open retained store must publish the unrelated root");
    };

    let dispatch_managed = Arc::clone(&managed);
    let dispatch = tokio::spawn(async move { dispatch_test_slot_hook(&dispatch_managed).await });
    managed.topology.hook_entered.notified().await;
    assert_eq!(
        managed.retained.replace(
            leased_id,
            Arc::new(Root {
                serial: 3,
                raw_drops: Arc::clone(&raw_drops),
            }),
        ),
        ReplaceStatus::Replaced
    );
    assert_eq!(
        managed.retained.replace(
            ready_id,
            Arc::new(Root {
                serial: 4,
                raw_drops: Arc::clone(&raw_drops),
            }),
        ),
        ReplaceStatus::Replaced
    );

    let first_destroy = managed.resource.destroy_completed.notified();
    tokio::pin!(first_destroy);
    first_destroy.as_mut().enable();
    let acquire = managed
        .run_acquire_loop(&test_context(), &AcquireOptions::default(), None)
        .await;
    assert_eq!(
        acquire
            .expect_err("test topology cannot create acquire entries")
            .kind(),
        &ErrorKind::Permanent
    );
    first_destroy.await;
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 1);

    let retirement_ready = managed.retained.wait_retired_ready();
    tokio::pin!(retirement_ready);
    assert!(
        futures::poll!(retirement_ready.as_mut()).is_pending(),
        "retirement readiness must be parked before the last lease is released"
    );
    let second_destroy = managed.resource.destroy_completed.notified();
    tokio::pin!(second_destroy);
    second_destroy.as_mut().enable();
    managed.topology.release_hook.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), retirement_ready)
        .await
        .expect("last lease release must promptly wake retirement readiness")
        .expect("released generation must become drainable");
    second_destroy.await;
    let dispatch_outcome = dispatch
        .await
        .expect("dispatch task must not panic")
        .expect("last lease drop must resume its blocked retirement");
    assert_eq!(dispatch_outcome, SubmissionOutcome::Completed);
    assert_eq!(managed.resource.destroyed.load(Ordering::SeqCst), 2);
    assert_eq!(raw_drops.load(Ordering::SeqCst), 2);
    assert_eq!(managed.release_queue.dropped_count(), 0);

    stop_queue(&managed, workers).await;
}

#[tokio::test]
async fn poisoned_retained_generation_does_not_skip_healthy_sibling_destruction() {
    let (managed, workers) = managed(QuiesceBehavior::Complete);
    let poisoned_drops = Arc::new(AtomicUsize::new(0));
    let poisoned_id = install_root(&managed, 1, Arc::clone(&poisoned_drops));
    managed.retained.poison_for_test(poisoned_id);
    let healthy_drops = Arc::new(AtomicUsize::new(0));
    let RetainStatus::Published(_) = managed.retained.retain(Arc::new(Root {
        serial: 2,
        raw_drops: Arc::clone(&healthy_drops),
    })) else {
        panic!("open retained store must publish the healthy sibling");
    };

    let outcome = managed.close_retained().await;

    let error = outcome.expect_err("poisoned accounting must fail closed after healthy cleanup");
    assert_eq!(error.kind(), &ErrorKind::Permanent);
    std::assert_matches!(
        std::error::Error::source(&error),
        Some(source) if source.is::<super::LeaseAccountingPoisoned>()
    );
    assert_eq!(
        managed.resource.destroyed.load(Ordering::SeqCst),
        1,
        "the healthy sibling must be destroyed exactly once"
    );
    assert_eq!(healthy_drops.load(Ordering::SeqCst), 1);
    assert_eq!(poisoned_drops.load(Ordering::SeqCst), 0);

    let (remaining_ready, remaining_blocked) = managed.retained.drain_all().into_parts();
    assert!(remaining_ready.is_empty());
    std::assert_matches!(
        remaining_blocked.map(crate::runtime::retained_store::DrainBlocked::reason),
        Some(
            crate::runtime::retained_store::DrainBlockReason::AccountingPoisoned {
                poisoned_entry_count: 1,
                live_lease_count: 0
            }
        )
    );
    stop_queue(&managed, workers).await;
}

#[tokio::test]
async fn cross_queue_credential_dispatch_reports_deferred_without_retrying() {
    let (managed, workers) = managed(QuiesceBehavior::Complete);
    let raw_drops = Arc::new(AtomicUsize::new(0));
    install_root(&managed, 1, raw_drops);
    let (caller_queue, caller_workers) = ReleaseQueue::new(1);
    let caller_queue = Arc::new(caller_queue);
    let dispatch_managed = Arc::clone(&managed);
    let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();

    let caller_submission = caller_queue
        .submit_release(move || {
            Box::pin(async move {
                let outcome = dispatch_test_slot_hook(&dispatch_managed).await?;
                outcome_tx.send(outcome).expect("outcome receiver is live");
                Ok(())
            })
        })
        .expect("open caller queue accepts dispatch");

    assert_eq!(
        outcome_rx.await.expect("dispatch reports acceptance"),
        SubmissionOutcome::Deferred,
    );
    assert_eq!(
        caller_submission.wait().await.unwrap(),
        SubmissionOutcome::Completed,
        "the caller task completes without treating accepted deferral as an error",
    );
    managed.topology.hook_entered.notified().await;
    assert_eq!(managed.topology.hook_completed.load(Ordering::SeqCst), 0);

    managed.topology.release_hook.notify_one();
    stop_queue(&managed, workers).await;
    assert_eq!(managed.topology.hook_completed.load(Ordering::SeqCst), 1);
    assert_eq!(managed.release_queue.dropped_count(), 0);

    caller_queue.close();
    ReleaseQueue::shutdown(caller_workers).await;
}
