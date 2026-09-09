use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use arc_swap::ArcSwap;
use futures::FutureExt;
use nebula_core::{ExecutionId, ResourceKey, resource_key};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::{
    context::ResourceContext,
    error::Error,
    recovery::gate::{RecoveryGate, RecoveryGateConfig},
    release_queue::ReleaseQueue,
    resource::{Provider, ResourceConfig, ResourceMetadata, TeardownCx},
    runtime::managed::ManagedResource,
    state::ResourceStatus,
    topology::{
        Pooled, Resident, pooled::config::Config as PoolConfig,
        resident::config::Config as ResidentConfig, store::InstanceStore,
    },
};

#[tokio::test]
async fn admitted_hook_deadline_defers_without_cancelling_queue_owned_task() {
    let (queue, workers) = ReleaseQueue::new(1);
    let worker_blocked = Arc::new(Notify::new());
    let release_worker = Arc::new(Notify::new());
    let worker_blocked_in_task = Arc::clone(&worker_blocked);
    let release_worker_in_task = Arc::clone(&release_worker);
    queue
        .submit_coordinator(move || {
            Box::pin(async move {
                worker_blocked_in_task.notify_one();
                release_worker_in_task.notified().await;
                Ok(())
            })
        })
        .expect("open queue accepts blocker")
        .detach();
    worker_blocked.notified().await;
    let hook_entered = Arc::new(Notify::new());
    let release_hook = Arc::new(Notify::new());
    let hook_runs = Arc::new(AtomicUsize::new(0));
    let observations = Arc::new(AtomicUsize::new(0));
    let (observation_tx, observation_rx) = tokio::sync::oneshot::channel();
    let observations_for_settlement = Arc::clone(&observations);
    let (settlement, admission) = SlotHookSettlement::new(Box::new(move |observation| {
        observations_for_settlement.fetch_add(1, Ordering::SeqCst);
        assert!(
            observation_tx.send(observation).is_ok(),
            "terminal observation receiver remains live"
        );
    }));
    let (hook_result_tx, hook_result_rx) = tokio::sync::oneshot::channel();
    let has_started = Arc::new(AtomicBool::new(false));
    let task_has_started = Arc::clone(&has_started);
    let hook_entered_in_task = Arc::clone(&hook_entered);
    let release_hook_in_task = Arc::clone(&release_hook);
    let hook_runs_in_task = Arc::clone(&hook_runs);
    let submission = queue
        .submit_coordinator(move || {
            Box::pin(async move {
                task_has_started.store(true, Ordering::Release);
                hook_runs_in_task.fetch_add(1, Ordering::SeqCst);
                hook_entered_in_task.notify_one();
                release_hook_in_task.notified().await;
                settlement.settle(SlotHookObservation::Completed);
                let _ = hook_result_tx.send(SlotHookWaitOutcome::Completed);
                Ok(())
            })
        })
        .expect("open queue accepts hook");
    submission.detach();
    admission.admit();
    let accepted = AcceptedSlotHook {
        receipt: SlotHookReceipt::Await(hook_result_rx),
        has_started,
    };

    assert!(matches!(
        accepted.wait_until(tokio::time::Instant::now()).await,
        SlotHookWaitOutcome::Deferred(SlotHookDeferral::ObservationTimedOut)
    ));
    release_worker.notify_one();
    hook_entered.notified().await;
    release_hook.notify_one();
    assert!(matches!(
        observation_rx.await.expect("queue task settles"),
        SlotHookObservation::Completed
    ));
    assert_eq!(hook_runs.load(Ordering::SeqCst), 1);
    assert_eq!(observations.load(Ordering::SeqCst), 1);

    queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test]
async fn closed_admitted_hook_receipt_is_terminal_abandonment_not_observer_deferral() {
    let (receipt_tx, receipt) = tokio::sync::oneshot::channel();
    drop(receipt_tx);
    let accepted = AcceptedSlotHook {
        receipt: SlotHookReceipt::Await(receipt),
        has_started: Arc::new(AtomicBool::new(true)),
    };

    assert!(matches!(
        accepted
            .wait_until(tokio::time::Instant::now() + std::time::Duration::from_mins(1))
            .await,
        SlotHookWaitOutcome::Abandoned
    ));
}

#[test]
fn retained_cleanup_settlement_publishes_each_fault_exactly_once() {
    let terminal_faults = [
        RetiredCleanupObservation::Failed(crate::ErrorKind::Permanent),
        RetiredCleanupObservation::TimedOut,
    ];
    for terminal_fault in terminal_faults {
        let observations = Arc::new(Mutex::new(Vec::new()));
        let observations_for_callback = Arc::clone(&observations);
        RetiredCleanupSettlement::new(Some(Box::new(move |observation| {
            observations_for_callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(observation);
        })))
        .settle(Some(terminal_fault.clone()));
        assert_eq!(
            *observations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            [terminal_fault]
        );
    }

    let observations = Arc::new(Mutex::new(Vec::new()));
    let observations_for_callback = Arc::clone(&observations);
    drop(RetiredCleanupSettlement::new(Some(Box::new(
        move |observation| {
            observations_for_callback
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(observation);
        },
    ))));
    assert_eq!(
        *observations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        [RetiredCleanupObservation::Abandoned]
    );
}

#[tokio::test]
async fn dropping_admitted_hook_waiter_preserves_exactly_once_terminal_settlement() {
    let (queue, workers) = ReleaseQueue::new(1);
    let hook_entered = Arc::new(Notify::new());
    let release_hook = Arc::new(Notify::new());
    let hook_runs = Arc::new(AtomicUsize::new(0));
    let observations = Arc::new(AtomicUsize::new(0));
    let (observation_tx, observation_rx) = tokio::sync::oneshot::channel();
    let observations_for_settlement = Arc::clone(&observations);
    let (settlement, admission) = SlotHookSettlement::new(Box::new(move |observation| {
        observations_for_settlement.fetch_add(1, Ordering::SeqCst);
        assert!(
            observation_tx.send(observation).is_ok(),
            "terminal observation receiver remains live"
        );
    }));
    let (hook_result_tx, hook_result_rx) = tokio::sync::oneshot::channel();
    let has_started = Arc::new(AtomicBool::new(false));
    let task_has_started = Arc::clone(&has_started);
    let hook_entered_in_task = Arc::clone(&hook_entered);
    let release_hook_in_task = Arc::clone(&release_hook);
    let hook_runs_in_task = Arc::clone(&hook_runs);
    let submission = queue
        .submit_coordinator(move || {
            Box::pin(async move {
                task_has_started.store(true, Ordering::Release);
                hook_runs_in_task.fetch_add(1, Ordering::SeqCst);
                hook_entered_in_task.notify_one();
                release_hook_in_task.notified().await;
                settlement.settle(SlotHookObservation::Completed);
                let _ = hook_result_tx.send(SlotHookWaitOutcome::Completed);
                Ok(())
            })
        })
        .expect("open queue accepts hook");
    submission.detach();
    admission.admit();
    let accepted = AcceptedSlotHook {
        receipt: SlotHookReceipt::Await(hook_result_rx),
        has_started,
    };

    hook_entered.notified().await;
    assert!(
        accepted
            .wait_until(tokio::time::Instant::now() + std::time::Duration::from_mins(1))
            .now_or_never()
            .is_none(),
        "blocked hook keeps the caller pending before cancellation"
    );
    release_hook.notify_one();
    assert!(matches!(
        observation_rx.await.expect("queue task settles"),
        SlotHookObservation::Completed
    ));
    assert_eq!(hook_runs.load(Ordering::SeqCst), 1);
    assert_eq!(observations.load(Ordering::SeqCst), 1);

    queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test(start_paused = true)]
async fn hook_receipt_settles_before_blocked_retained_cleanup() {
    let resource = RetainedHookMock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let destroy_finished = Arc::clone(&resource.destroy_finished);
    let (queue, workers) = ReleaseQueue::new(1);
    let queue = Arc::new(queue);
    let managed = Arc::new(ManagedResource {
        resource,
        config: ArcSwap::from_pointee(PoolCfg),
        topology: Resident::<RetainedHookMock>::new(ResidentConfig::default()),
        store: InstanceStore::with_abandonment_tracker(None, queue.abandonment_tracker()),
        retained: crate::RetainedStore::new(queue.abandonment_tracker()),
        release_queue: Arc::clone(&queue),
        generation: AtomicU64::new(0),
        status: ArcSwap::from_pointee(ResourceStatus::new()),
        recovery_gate: None,
        tainted: AtomicBool::new(false),
        in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
        maintenance_sweeps: AtomicU64::new(0),
        maintenance: Default::default(),
    });

    let displaced = Arc::new(0);
    let crate::RetainStatus::Published(retained_id) = managed.retained.retain(displaced) else {
        panic!("open retained store must publish the generation")
    };
    let retained_lease = managed
        .retained
        .lease(retained_id)
        .expect("published generation is leasable");
    let replacement = Arc::new(1);
    assert_eq!(
        managed.retained.replace(retained_id, replacement),
        crate::ReplaceStatus::Replaced
    );

    let terminal_observations = Arc::new(AtomicUsize::new(0));
    let cleanup_failures = Arc::new(AtomicUsize::new(0));
    let terminal_observations_for_callback = Arc::clone(&terminal_observations);
    let cleanup_failures_for_callback = Arc::clone(&cleanup_failures);
    let (settlement, admission) = SlotHookSettlement::new(Box::new(move |observation| {
        assert!(matches!(observation, SlotHookObservation::Completed));
        terminal_observations_for_callback.fetch_add(1, Ordering::SeqCst);
    }));
    let settlement = settlement.with_cleanup_observer(Box::new(move |_| {
        cleanup_failures_for_callback.fetch_add(1, Ordering::SeqCst);
    }));
    let accepted = Arc::clone(&managed)
        .submit_slot_hook(
            "credential",
            true,
            std::time::Duration::from_secs(5),
            settlement,
            admission,
        )
        .expect("open queue accepts the hook");

    assert!(matches!(
        accepted
            .wait_until(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await,
        SlotHookWaitOutcome::Completed
    ));
    assert_eq!(terminal_observations.load(Ordering::SeqCst), 1);
    assert_eq!(destroyed.load(Ordering::SeqCst), 0);

    drop(retained_lease);
    destroy_finished.notified().await;
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(cleanup_failures.load(Ordering::SeqCst), 0);

    queue.close();
    ReleaseQueue::shutdown(workers).await;
}

// Minimal pooled resource config used by both test helpers.
#[derive(Clone)]
struct PoolCfg;
crate::impl_empty_has_schema!(PoolCfg);
impl ResourceConfig for PoolCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct Mock {
    created: Arc<AtomicU64>,
    destroyed: Arc<AtomicU64>,
    /// When `true`, `check` parks forever — the deterministic suspension
    /// point for the accept-await cancellation tests.
    hang_check: Arc<AtomicBool>,
    /// Probe-lock regression fixture: when `true`, `check` notifies
    /// `check_started` the instant it begins, then parks on
    /// `release_check` until the test releases it — a *slow* (it
    /// eventually resolves) check, distinct from `hang_check` (never
    /// resolves). Lets a test observe "the probe is mid-check, still
    /// outside the idle lock" deterministically.
    park_in_check: Arc<AtomicBool>,
    check_started: Arc<Notify>,
    release_check: Arc<Notify>,
    /// Min-idle-refill fixture: when `true`, the *next* `create` call notifies
    /// `create_entered` the instant it begins, then parks on
    /// `release_create` until the test releases it — lets a test observe
    /// "a create is in flight, entry not yet deposited" deterministically
    /// (mirrors `park_in_check`, but for `create` rather than `check`).
    park_create: Arc<AtomicBool>,
    create_entered: Arc<Notify>,
    release_create: Arc<Notify>,
    batch_failures: Arc<AtomicBool>,
    dependent_release: Arc<Mutex<Option<ResourceGuard<Self>>>>,
    release_dependency_in_check: Arc<AtomicBool>,
    fail_probe_batch_then_park: Arc<AtomicBool>,
    destroy_finished: Arc<Notify>,
}

impl Mock {
    fn new() -> Self {
        Self {
            created: Arc::new(AtomicU64::new(0)),
            destroyed: Arc::new(AtomicU64::new(0)),
            hang_check: Arc::new(AtomicBool::new(false)),
            park_in_check: Arc::new(AtomicBool::new(false)),
            check_started: Arc::new(Notify::new()),
            release_check: Arc::new(Notify::new()),
            park_create: Arc::new(AtomicBool::new(false)),
            create_entered: Arc::new(Notify::new()),
            release_create: Arc::new(Notify::new()),
            batch_failures: Arc::new(AtomicBool::new(false)),
            dependent_release: Arc::default(),
            release_dependency_in_check: Arc::default(),
            fail_probe_batch_then_park: Arc::default(),
            destroy_finished: Arc::default(),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Mock {
    type Config = PoolCfg;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("acquire-loop-guard-mock")
    }

    async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        let id = self.created.fetch_add(1, Ordering::SeqCst);
        if self.park_create.swap(false, Ordering::SeqCst) {
            self.create_entered.notify_one();
            self.release_create.notified().await;
        }
        Ok(id)
    }

    async fn check(&self, runtime: &u64) -> Result<(), Error> {
        if self.fail_probe_batch_then_park.load(Ordering::SeqCst) {
            if *runtime < PROBE_CONCURRENCY as u64 {
                return Err(Error::permanent("intentional failed first probe batch"));
            }
            self.check_started.notify_one();
            self.release_check.notified().await;
        }
        if self.hang_check.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
            // guard-justified: `std::future::pending()` never resolves,
            // so this line is statically unreachable.
            unreachable!("pending future never resolves")
        }
        if self.park_in_check.load(Ordering::SeqCst) {
            self.check_started.notify_one();
            self.release_check.notified().await;
        }
        if self.release_dependency_in_check.load(Ordering::SeqCst) {
            let child = self.dependent_release.lock().unwrap().take();
            if let Some(child) = child {
                let _release_outcome = child.release().await?;
            }
        }
        Ok(())
    }

    async fn destroy(&self, runtime: u64, _cx: TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        if self.batch_failures.load(Ordering::SeqCst) {
            match runtime {
                0 => std::future::pending::<()>().await,
                1 => return Err(Error::permanent("intentional batch member failure")),
                3 => panic!("intentional batch member panic"),
                _ => {},
            }
        }
        if runtime == 0 {
            let child = self.dependent_release.lock().unwrap().take();
            if let Some(child) = child {
                let _release_outcome = child.release().await?;
            }
            self.destroy_finished.notify_one();
        }
        Ok(())
    }

    fn teardown_budget(&self) -> std::time::Duration {
        if self.batch_failures.load(Ordering::SeqCst) {
            std::time::Duration::from_hours(1)
        } else {
            std::time::Duration::from_secs(30)
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

crate::no_credential_slots!(Mock);

impl crate::topology::pooled::PoolProvider for Mock {}

#[derive(Clone)]
struct RetainedHookMock {
    destroyed: Arc<AtomicU64>,
    destroy_finished: Arc<Notify>,
}

impl RetainedHookMock {
    fn new() -> Self {
        Self {
            destroyed: Arc::new(AtomicU64::new(0)),
            destroy_finished: Arc::new(Notify::new()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for RetainedHookMock {
    type Config = PoolCfg;
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("retained-hook-settlement-mock")
    }

    async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(0)
    }

    async fn destroy(&self, _runtime: u64, _cx: TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        self.destroy_finished.notify_one();
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

crate::no_credential_slots!(RetainedHookMock);
impl crate::topology::resident::ResidentProvider for RetainedHookMock {}

fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

fn managed(resource: Mock, config: PoolConfig) -> Arc<ManagedResource<Mock>> {
    let (rq, _handle) = ReleaseQueue::new(1);
    let topology = Pooled::<Mock>::new(config, 0);
    Arc::new(ManagedResource {
        resource,
        config: ArcSwap::from_pointee(PoolCfg),
        topology,
        store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
        retained: crate::RetainedStore::new(rq.abandonment_tracker()),
        release_queue: Arc::new(rq),
        generation: AtomicU64::new(0),
        status: ArcSwap::from_pointee(ResourceStatus::new()),
        recovery_gate: None,
        tainted: AtomicBool::new(false),
        in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
        maintenance_sweeps: AtomicU64::new(0),
        maintenance: Default::default(),
    })
}

#[path = "tests/batch.rs"]
mod batch_tests;

#[path = "tests/maintenance.rs"]
mod maintenance_tests;

#[path = "tests/custom_fence.rs"]
mod custom_fence_tests;

/// Cancel-safety regression (audit 2026-07-01 bug #1): an acquire future
/// cancelled while suspended in `Topology::accept` (here: a hanging
/// `test_on_checkout` health check) must destroy the popped idle entry via
/// the release queue — before the fix the entry was a plain local across
/// the `accept().await` and a cancellation dropped the live instance
/// without ever calling `Provider::destroy` (permanent leak: the entry was
/// already off the idle queue).
#[tokio::test]
async fn cancelled_acquire_during_accept_destroys_the_popped_entry() {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let hang_check = Arc::clone(&resource.hang_check);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let mr = {
        let topology = Pooled::<Mock>::new(
            PoolConfig {
                test_on_checkout: true,
                ..PoolConfig::default()
            },
            0,
        );
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
            retained: crate::RetainedStore::new(rq.abandonment_tracker()),
            release_queue: Arc::clone(&rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: None,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
            maintenance: Default::default(),
        })
    };

    // Seed one healthy idle entry, then arm the hang so the NEXT acquire
    // parks inside `accept`'s health check with the entry popped.
    let entry = mr
        .topology
        .create_entry(&mr.resource, &PoolCfg, &test_ctx(), &mr.retained)
        .await
        .expect("create the seed entry");
    let entry = entry.into_entry();
    assert!(mr.retained.drain_retired().is_empty());
    let epoch = mr.store.stamp_epoch();
    assert!(
        !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
        "the seed entry must land in the idle queue"
    );
    hang_check.store(true, Ordering::SeqCst);

    // The cancellation: a timeout drops the acquire future while it is
    // suspended in `accept` → `resource.check`.
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        mr.run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None),
    )
    .await;
    assert!(
        cancelled.is_err(),
        "the acquire must still be parked in the hanging health check \
             when the timeout fires"
    );
    assert!(
        mr.store.is_empty().await,
        "the popped entry must not have been silently re-queued"
    );

    // Drain the release queue and assert the destroy actually ran.
    rq.close();
    drop(rq);
    drop(mr);
    ReleaseQueue::shutdown(rq_handle).await;
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "a cancellation during `accept` must destroy the popped entry via \
             the ReleaseQueue, never leak it through a plain Drop"
    );
}

/// Cancel-safety regression (audit 2026-07-01 bug #2): a warmup future
/// dropped between `create_entry` succeeding and the fenced deposit
/// completing (here: parked on the held idle lock; in production the
/// author-hook ceiling timeout in `Manager::warmup_pool`) must destroy
/// the created instance via the release queue — before the fix the entry
/// travelled unguarded into `deposit_fresh`'s future and a cancellation
/// dropped it without `Provider::destroy`.
#[tokio::test]
async fn cancelled_warmup_between_create_and_deposit_destroys_the_entry() {
    let resource = Mock::new();
    let created = Arc::clone(&resource.created);
    let destroyed = Arc::clone(&resource.destroyed);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let mr = {
        let topology = Pooled::<Mock>::new(
            PoolConfig {
                min_size: 1, // warmup_target = 1
                ..PoolConfig::default()
            },
            0,
        );
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
            retained: crate::RetainedStore::new(rq.abandonment_tracker()),
            release_queue: Arc::clone(&rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: None,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
            maintenance: Default::default(),
        })
    };

    // Hold the idle lock so warmup creates its entry, then parks on the
    // lock acquisition — the exact created-but-undeposited window.
    let idle_lock = mr.store.lock_idle().await;
    let ctx = test_ctx();
    {
        let mut warmup = Box::pin(mr.warmup(&ctx));
        let parked = tokio::time::timeout(std::time::Duration::from_millis(100), &mut warmup).await;
        assert!(
            parked.is_err(),
            "warmup must be parked awaiting the idle lock with a created entry in hand"
        );
        drop(warmup); // the cancellation
    }
    drop(idle_lock);

    assert_eq!(
        created.load(Ordering::SeqCst),
        1,
        "exactly one instance was created before the cancellation"
    );
    assert!(
        mr.store.is_empty().await,
        "the cancelled warmup must not have deposited the entry"
    );

    rq.close();
    drop(rq);
    drop(mr);
    ReleaseQueue::shutdown(rq_handle).await;
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "a warmup cancelled between create and deposit must destroy the \
             created instance via the ReleaseQueue, never leak it"
    );
}

/// Cancel-safety: an [`EntryCreateGuard`] dropped before `defuse` schedules an
/// async `destroy` via the release queue.
#[tokio::test]
async fn entry_create_guard_drop_destroys_via_release_queue() {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let mr = {
        let topology = Pooled::<Mock>::new(PoolConfig::default(), 0);
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
            retained: crate::RetainedStore::new(rq.abandonment_tracker()),
            release_queue: Arc::clone(&rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: None,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
            maintenance: Default::default(),
        })
    };

    let entry = mr
        .topology
        .create_entry(&mr.resource, &PoolCfg, &test_ctx(), &mr.retained)
        .await
        .expect("create");
    let entry = entry.into_entry();
    assert!(mr.retained.drain_retired().is_empty());
    let guard = EntryCreateGuard::new(entry, Arc::clone(&mr), Arc::clone(&rq));
    // Simulate a cancelled acquire: dropped before `defuse`.
    drop(guard);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // Signal the workers to drain + exit before joining. `drop(rq)` alone
    // does NOT close the channels here: `mr` holds another
    // `Arc<ReleaseQueue>`, so the senders outlive the test's `rq` and the
    // worker loop would block on `rx.recv()` forever. `close()` cancels the
    // token, the documented precondition for `shutdown`.
    rq.close();
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;

    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "EntryCreateGuard::drop must schedule destroy via the ReleaseQueue \
             when the acquire future is cancelled mid-create"
    );
}

/// A `EntryCreateGuard` that runs through `defuse` (the success path) must
/// NOT trigger a stray destroy.
#[tokio::test]
async fn entry_create_guard_defuse_skips_destroy() {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let mr = managed(resource, PoolConfig::default());

    let entry = mr
        .topology
        .create_entry(&mr.resource, &PoolCfg, &test_ctx(), &mr.retained)
        .await
        .expect("create");
    let entry = entry.into_entry();
    assert!(mr.retained.drain_retired().is_empty());
    let guard = EntryCreateGuard::new(entry, Arc::clone(&mr), Arc::clone(&rq));
    let _entry = guard.defuse();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    drop(rq);
    ReleaseQueue::shutdown(rq_handle).await;

    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        0,
        "a defused EntryCreateGuard must not schedule a destroy"
    );
}

// ── Probe-lock fix regression tests ─────────────────────────────────

/// A slow (but eventually healthy) `Provider::check` must not block a
/// concurrent checkout while a maintenance probe is running — the idle
/// lock is held only to drain the queue, never across the check itself.
#[tokio::test]
async fn probe_slow_check_does_not_block_concurrent_checkout() {
    let resource = Mock::new();
    let mr = managed(resource.clone(), PoolConfig::default());

    // Seed one idle entry for the probe to find.
    let entry = mr
        .topology
        .create_entry(&mr.resource, &PoolCfg, &test_ctx(), &mr.retained)
        .await
        .expect("create the seed entry");
    let entry = entry.into_entry();
    assert!(mr.retained.drain_retired().is_empty());
    let epoch = mr.store.stamp_epoch();
    assert!(
        !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
        "the seed entry must land in the idle queue"
    );

    resource.park_in_check.store(true, Ordering::SeqCst);
    let check_started = Arc::clone(&resource.check_started);
    let release_check = Arc::clone(&resource.release_check);

    let mr_probe = Arc::clone(&mr);
    let probe_task = tokio::spawn(async move { mr_probe.probe_idle_entries().await });

    // Deterministic: the probe has drained the store and its (only)
    // `check()` call has started — i.e. it is now suspended OUTSIDE the
    // idle lock (the fix under test). Pre-fix, the lock was held across
    // this exact suspension point.
    check_started.notified().await;

    // Prove the idle lock is free: a concurrent checkout completes
    // promptly instead of blocking on the still-in-flight probe. It
    // correctly observes an empty queue (the probe drained the only
    // entry) — the point is that it does not HANG waiting for a lock.
    let checkout = tokio::time::timeout(std::time::Duration::from_millis(200), mr.store.checkout())
        .await
        .expect(
            "checkout must not block on a slow probe check — the idle \
                 lock must not be held across it",
        );
    assert!(
        checkout.fresh.is_none(),
        "the single idle entry is off-store while the probe holds it mid-check"
    );

    // Release the check and let the probe finish.
    release_check.notify_one();
    let failed = probe_task.await.expect("probe task must not panic");
    assert!(
        failed.is_empty(),
        "the slow-but-healthy check must survive, not be marked failed"
    );
    assert_eq!(
        mr.store.len().await,
        1,
        "the survivor must be returned to the idle queue via the \
             epoch-fenced return path"
    );
}

/// A probe sweep must drain the idle store in batches of at most
/// [`PROBE_CONCURRENCY`], never the whole idle queue in one shot — the
/// bound on how far live instances can transiently overshoot the
/// topology's cap while a sweep is in flight (a concurrent acquire can
/// create a fresh instance for each entry currently drained-but-not-yet-
/// returned).
#[tokio::test]
async fn probe_drains_in_bounded_batches_not_the_whole_idle_queue() {
    let resource = Mock::new();
    let mr = managed(resource.clone(), PoolConfig::default());

    // Seed more idle entries than a single probe batch holds, so the
    // batch boundary is observable.
    let seeded = PROBE_CONCURRENCY + 2;
    for _ in 0..seeded {
        let entry = mr
            .topology
            .create_entry(&mr.resource, &PoolCfg, &test_ctx(), &mr.retained)
            .await
            .expect("create seed entry");
        let entry = entry.into_entry();
        assert!(mr.retained.drain_retired().is_empty());
        let epoch = mr.store.stamp_epoch();
        assert!(
            !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
            "every seed entry must land in the idle queue"
        );
    }
    assert_eq!(mr.store.len().await, seeded);

    resource.park_in_check.store(true, Ordering::SeqCst);
    let check_started = Arc::clone(&resource.check_started);
    let release_check = Arc::clone(&resource.release_check);

    let mr_probe = Arc::clone(&mr);
    let probe_task = tokio::spawn(async move { mr_probe.probe_idle_entries().await });

    // Deterministic: the first batch's checks have started, which only
    // happens after that batch's drain (under the idle lock) already
    // completed.
    check_started.notified().await;

    // The bounded-batch drain must leave the rest of the idle queue
    // alone — never the whole-queue drain a single unbounded `mem::take`
    // would perform.
    assert_eq!(
        mr.store.len().await,
        seeded - PROBE_CONCURRENCY,
        "one probe batch must drain at most PROBE_CONCURRENCY entries, \
             leaving the rest of the idle queue available to concurrent \
             acquires instead of the whole queue at once"
    );

    // Release the first batch and let the remaining batch(es) proceed
    // without parking, so the sweep can finish.
    resource.park_in_check.store(false, Ordering::SeqCst);
    release_check.notify_waiters();

    let failed = probe_task.await.expect("probe task must not panic");
    assert!(
        failed.is_empty(),
        "every seeded entry is healthy and must survive the sweep"
    );
    assert_eq!(
        mr.store.len().await,
        seeded,
        "every entry must be returned to the idle queue across every batch"
    );
}

/// A credential revoke that lands WHILE an entry is mid-probe (drained,
/// health check in flight) must destroy that entry on return — never
/// re-admit it to the idle queue. This is the fence-preservation half of
/// the probe-lock fix: a plain `*idle = survivors` write-back would
/// resurrect a since-revoked entry; routing survivors back through
/// `InstanceStore::return_entry` re-checks the epoch under the re-taken
/// lock and evicts instead.
#[tokio::test]
async fn probe_revoke_mid_probe_destroys_probed_entries_not_redeposited() {
    let resource = Mock::new();
    let destroyed = Arc::clone(&resource.destroyed);
    let mr = managed(resource.clone(), PoolConfig::default());

    let entry = mr
        .topology
        .create_entry(&mr.resource, &PoolCfg, &test_ctx(), &mr.retained)
        .await
        .expect("create the seed entry");
    let entry = entry.into_entry();
    assert!(mr.retained.drain_retired().is_empty());
    let epoch = mr.store.stamp_epoch();
    assert!(
        !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
        "the seed entry must land in the idle queue"
    );

    resource.park_in_check.store(true, Ordering::SeqCst);
    let check_started = Arc::clone(&resource.check_started);
    let release_check = Arc::clone(&resource.release_check);

    let mr_probe = Arc::clone(&mr);
    let probe_task = tokio::spawn(async move { mr_probe.probe_idle_entries().await });

    check_started.notified().await;

    // The revoke fence bump — exactly what `Manager::revoke_slot`'s
    // synchronous phase 1 does — lands while the entry is drained and
    // mid-check, strictly BEFORE the check resolves.
    mr.store.bump_revoke_epoch();

    // Let the (otherwise healthy) check resolve.
    release_check.notify_one();
    let failed = probe_task.await.expect("probe task must not panic");

    assert_eq!(
        failed.len(),
        1,
        "an entry revoked mid-probe must be reported for the caller to \
             destroy, not silently dropped or kept"
    );
    assert_eq!(
        mr.store.len().await,
        0,
        "an entry revoked mid-probe must NEVER be written back to the \
             idle queue (a plain `*idle = survivors` write-back would \
             resurrect a revoked entry)"
    );

    // Run the destroy the caller (`run_maintenance`, in production)
    // performs on every `failed` entry, and confirm it actually ran —
    // proving this is a real destroy path, not just an accounting
    // artifact.
    assert_eq!(
        mr.release_queue
            .submit_coordinator(move || Box::pin(failed.run()))
            .expect("open queue must accept test cleanup")
            .wait()
            .await
            .unwrap(),
        SubmissionOutcome::Completed,
    );
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "the revoked-mid-probe entry must actually be torn down via \
             Provider::destroy"
    );
}

// ── Reaper-tick min-idle floor refill ───────────────────────────────

/// After a maintenance sweep evicts every idle entry, `refill_min_idle`
/// tops the store back up to `min_size` (warmup_target) — the reaper
/// closes the gap proactively instead of waiting for the next
/// caller-driven acquire to create one on demand.
#[tokio::test]
async fn refill_min_idle_tops_up_after_maintenance_eviction() {
    let resource = Mock::new();
    let created = Arc::clone(&resource.created);
    let mr = managed(
        resource,
        PoolConfig {
            min_size: 2,
            max_size: 4,
            idle_timeout: None,
            max_lifetime: None,
            ..PoolConfig::default()
        },
    );

    // Two overlapping leases so both entries land in the idle queue on
    // release — a serial acquire-release would just reuse the one entry
    // and only ever accumulate one.
    let g1 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("acquire 1");
    let g2 = mr
        .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
        .await
        .expect("acquire 2");
    assert_eq!(
        g1.release().await.expect("release 1"),
        crate::ReleaseOutcome::Completed
    );
    assert_eq!(
        g2.release().await.expect("release 2"),
        crate::ReleaseOutcome::Completed
    );
    assert_eq!(mr.store.len().await, 2);

    // Force eviction deterministically via a fingerprint bump (no
    // wall-clock sleep needed).
    mr.set_fingerprint(99);
    let evicted = mr.run_maintenance().await;
    assert_eq!(evicted, 2, "both stale-fingerprint entries must be evicted");
    assert_eq!(mr.store.len().await, 0);

    let refilled = mr.refill_min_idle(&test_ctx()).await;
    assert_eq!(
        refilled, 2,
        "refill must top the idle queue back up to min_size"
    );
    assert_eq!(mr.store.len().await, 2);
    assert_eq!(
        created.load(Ordering::SeqCst),
        4,
        "2 initial creates + 2 refill creates"
    );
}

/// MAJOR regression (final-review item 2): the deficit computation used
/// to read only the idle-queue floor (`warmup_target - idle_len`),
/// ignoring currently checked-out leases tracked by
/// `ManagedResource::in_flight`. Under full load — idle empty, every
/// permit already leased — the naive deficit equalled `min_size` and the
/// tick created that many *extra* instances on top of the `max_size`
/// already checked out, overshooting the pool to `max_size + min_size`
/// live instances. The fix additionally bounds the refill by
/// `store.capacity() - (idle_len + in_flight)`; at `max_size` fully
/// leased that headroom is zero, so the tick must create nothing.
#[tokio::test]
async fn refill_min_idle_does_not_overshoot_when_pool_is_fully_leased() {
    let resource = Mock::new();
    let created = Arc::clone(&resource.created);
    let (rq, _handle) = ReleaseQueue::new(1);
    let mr = {
        let config = PoolConfig {
            min_size: 2,
            max_size: 2,
            idle_timeout: None,
            max_lifetime: None,
            ..PoolConfig::default()
        };
        let topology = Pooled::<Mock>::new(config.clone(), 0);
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::with_abandonment_tracker(
                Some(config.max_size as usize),
                rq.abandonment_tracker(),
            ),
            retained: crate::RetainedStore::new(rq.abandonment_tracker()),
            release_queue: Arc::new(rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: None,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
            maintenance: Default::default(),
        })
    };

    // Simulate both `max_size` leases already checked out and in flight
    // — the state `Manager::acquire_pooled`'s `InFlightCounter` puts
    // `ManagedResource::in_flight` in for the duration of a lease.
    // `run_acquire_loop` alone (used directly by this module's unit
    // tests) never touches that counter, so it is set directly here to
    // isolate `refill_min_idle`'s bound math from the manager dispatch
    // layer.
    mr.in_flight.0.store(2, Ordering::SeqCst);
    assert_eq!(
        mr.store.len().await,
        0,
        "idle queue starts empty — both entries are checked out, not idle"
    );

    let refilled = mr.refill_min_idle(&test_ctx()).await;
    assert_eq!(
        refilled, 0,
        "no headroom left under max_size while both leases are in flight \
             — the reaper tick must be a no-op"
    );
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "refill must not create instances beyond max_size"
    );
}

/// A `RecoveryGate` in any state other than `Idle` (a recovery attempt
/// in progress here) must make `refill_min_idle` a complete no-op —
/// creating replacement entries against a backend the gate has already
/// flagged unhealthy would recreate the exact stampede the gate exists
/// to prevent.
#[tokio::test]
async fn refill_min_idle_skips_when_gate_not_idle() {
    let resource = Mock::new();
    let created = Arc::clone(&resource.created);
    let gate = RecoveryGate::new(RecoveryGateConfig::default());
    // Holding the ticket moves the gate to `InProgress`.
    let _ticket = gate.try_begin().expect("gate starts idle");

    let mr = {
        let (rq, _handle) = ReleaseQueue::new(1);
        let topology = Pooled::<Mock>::new(
            PoolConfig {
                min_size: 2,
                ..PoolConfig::default()
            },
            0,
        );
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
            retained: crate::RetainedStore::new(rq.abandonment_tracker()),
            release_queue: Arc::new(rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: Some(Arc::new(gate)),
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
            maintenance: Default::default(),
        })
    };

    let refilled = mr.refill_min_idle(&test_ctx()).await;
    assert_eq!(
        refilled, 0,
        "a non-Idle gate must skip the refill tick entirely"
    );
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "no create_entry call must happen while the gate is not Idle"
    );
}

/// Cancel-safety invariant: shutdown-during-refill race is clean. A refill task
/// aborted while `create` is in flight (before the entry is deposited —
/// here: parked in the mock's `create`; in production the reaper task
/// being cancelled by `graceful_shutdown`) must destroy the created
/// instance via the release queue, never leak it and never panic —
/// mirrors `cancelled_warmup_between_create_and_deposit_destroys_the_entry`,
/// proving `refill_min_idle` inherited the same cancel-safety contract
/// through the shared `create_and_deposit_entries` helper.
#[tokio::test]
async fn refill_min_idle_shutdown_race_destroys_in_flight_entry() {
    let resource = Mock::new();
    resource.park_create.store(true, Ordering::SeqCst);
    let created = Arc::clone(&resource.created);
    let destroyed = Arc::clone(&resource.destroyed);
    let create_entered = Arc::clone(&resource.create_entered);
    let release_create = Arc::clone(&resource.release_create);
    let (rq, rq_handle) = ReleaseQueue::new(1);
    let rq = Arc::new(rq);
    let mr = {
        let topology = Pooled::<Mock>::new(
            PoolConfig {
                min_size: 1, // refill deficit == 1 against an empty store
                ..PoolConfig::default()
            },
            0,
        );
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::with_abandonment_tracker(None, rq.abandonment_tracker()),
            retained: crate::RetainedStore::new(rq.abandonment_tracker()),
            release_queue: Arc::clone(&rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: None,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
            maintenance: Default::default(),
        })
    };

    let mr_refill = Arc::clone(&mr);
    let ctx = test_ctx();
    let refill_task = tokio::spawn(async move { mr_refill.refill_min_idle(&ctx).await });

    // `len()` (the deficit check) strictly precedes `create_entry` in
    // program order, so by the time `create` has entered and parked, the
    // idle lock is free — take it ourselves *before* letting `create`
    // resume, so the loop's post-create `lock_idle().await` blocks on
    // us. This produces the exact created-but-undeposited window a
    // cancelled reaper task (shutdown) lands in, without racing `len()`
    // for the same lock (holding it from the start would block `len()`
    // itself, never reaching `create_entry` at all).
    create_entered.notified().await;
    let idle_lock = mr.store.lock_idle().await;
    release_create.notify_one();
    // Let the task resume past `create`, build its `EntryCreateGuard`,
    // and block on the lock we hold. Single-threaded test runtime: this
    // only needs to yield long enough for the scheduler to poll the
    // parked task once.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    refill_task.abort(); // the cancellation: shutdown drops the reaper task
    let _ = refill_task.await; // best-effort join (JoinError::is_cancelled())
    drop(idle_lock);

    assert_eq!(
        created.load(Ordering::SeqCst),
        1,
        "exactly one instance was created before the cancellation"
    );
    assert!(
        mr.store.is_empty().await,
        "the cancelled refill must not have deposited the entry"
    );

    rq.close();
    drop(rq);
    drop(mr);
    ReleaseQueue::shutdown(rq_handle).await;
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "a refill cancelled between create and deposit must destroy the \
             created instance via the ReleaseQueue, never leak it — no panic, \
             no leak"
    );
}

/// Fence invariant: revoke-during-refill. A `revoke_slot` epoch bump that
/// lands while a refill-created entry is still mid-`create` (the window
/// between the epoch snapshot and the fenced deposit) must make the
/// deposit fence destroy the entry, never admit it to the idle queue as
/// a since-revoked instance.
#[tokio::test]
async fn refill_min_idle_revoke_mid_create_destroys_not_deposits() {
    let resource = Mock::new();
    resource.park_create.store(true, Ordering::SeqCst);
    let created = Arc::clone(&resource.created);
    let destroyed = Arc::clone(&resource.destroyed);
    let create_entered = Arc::clone(&resource.create_entered);
    let release_create = Arc::clone(&resource.release_create);
    let mr = managed(
        resource,
        PoolConfig {
            min_size: 1,
            ..PoolConfig::default()
        },
    );

    let mr_refill = Arc::clone(&mr);
    let ctx = test_ctx();
    let refill_task = tokio::spawn(async move { mr_refill.refill_min_idle(&ctx).await });

    // Deterministic: the entry's pre-revoke epoch is already snapshotted
    // (`create_and_deposit_entries` stamps it before calling
    // `create_entry`) and `create` is now parked mid-flight.
    create_entered.notified().await;

    // The revoke fence bump — exactly what `Manager::revoke_slot`'s
    // synchronous phase 1 does — lands while the entry is still being
    // created, strictly before the fenced deposit.
    mr.bump_revoke_epoch();
    release_create.notify_one();

    // Let the refill actually finish: it must observe the epoch
    // mismatch at deposit and destroy the entry instead of admitting it.
    let refilled = refill_task.await.expect("refill task must not panic");
    assert_eq!(
        refilled, 0,
        "the epoch-fenced entry must not count as a successful refill"
    );
    assert!(
        mr.store.is_empty().await,
        "the deposit fence must never admit a since-revoked entry — no \
             plain write-back that would resurrect it"
    );
    assert_eq!(created.load(Ordering::SeqCst), 1);
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "the fenced entry must be destroyed, not silently dropped"
    );
}
