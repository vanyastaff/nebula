//! Fault injection at the private publisher seam; public behavior uses real leases.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::Notify;

use super::{
    DrainTimeoutPolicy, Manager, ManagerConfig, RegistrationSpec, ShutdownConfig, ShutdownError,
};
use crate::{
    Error, ResourceContext, ResourceGuard, RetainStatus, RetainedStore, ScopeLevel, SlotIdentity,
    TeardownCx,
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::{CreatedEntry, Ticket, Topology, Unavailable, store::InstanceStore},
};

#[test]
fn shutdown_timeout_diagnostic_covers_all_terminal_stages() {
    let error = ShutdownError::ReleaseQueueTimeout {
        timeout: Duration::from_secs(3),
    };
    assert_eq!(
        error.to_string(),
        "resource shutdown did not complete within its configured deadline (cleanup budget: 3s)"
    );
}

#[test]
fn shutdown_task_failure_diagnostic_does_not_misattribute_the_component() {
    assert_eq!(
        ShutdownError::RetirementSupervisorFailed.to_string(),
        "resource shutdown task failed before completing terminal work"
    );
}

#[derive(Clone, nebula_schema::Schema)]
struct Config;
impl ResourceConfig for Config {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct LeasedResource {
    destroyed: Arc<AtomicUsize>,
}
crate::no_credential_slots!(LeasedResource);

#[async_trait::async_trait]
impl Provider for LeasedResource {
    type Config = Config;
    type Instance = AtomicUsize;
    type Topology = QuiescingTopology;
    fn key() -> nebula_core::ResourceKey {
        nebula_core::resource_key!("shutdown-session-lease")
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("shutdown-session-lease"),
            "",
        )
    }
    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<AtomicUsize, Error> {
        Ok(AtomicUsize::new(7))
    }
    async fn destroy(&self, _: AtomicUsize, _: TeardownCx) -> Result<(), Error> {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct QuiescingTopology {
    quiesced: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    retained_id: tokio::sync::Mutex<Option<crate::RetainedId>>,
}

impl Topology<LeasedResource> for QuiescingTopology {
    type Entry = Arc<AtomicUsize>;
    fn try_reserve(&self, _: &InstanceStore<Self::Entry>) -> Result<Ticket, Unavailable> {
        Ok(Ticket::infallible())
    }
    async fn create_entry(
        &self,
        resource: &LeasedResource,
        config: &Config,
        context: &ResourceContext,
        retained: &RetainedStore<Self::Entry>,
    ) -> Result<CreatedEntry<Self::Entry>, Error> {
        let mut retained_id = self.retained_id.lock().await;
        if let Some(lease) = retained_id.and_then(|id| retained.lease(id)) {
            return Ok(CreatedEntry::new(Arc::clone(&lease)));
        }
        let entry = Arc::new(resource.create(config, context).await?);
        match retained.retain(Arc::clone(&entry)) {
            RetainStatus::Published(id) => {
                *retained_id = Some(id);
                Ok(CreatedEntry::new(entry))
            },
            RetainStatus::Retired(_) => Err(Error::cancelled()),
        }
    }
    fn entry_instance<'entry>(&self, entry: &'entry Self::Entry) -> &'entry AtomicUsize {
        entry
    }
    fn into_owned_instance(&self, entry: Self::Entry) -> Option<AtomicUsize> {
        Arc::into_inner(entry)
    }
    async fn quiesce(&self) -> Result<(), Error> {
        self.quiesced.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        Ok(())
    }
}

struct Fixture {
    manager: Manager,
    guard: ResourceGuard<LeasedResource>,
    destroyed: Arc<AtomicUsize>,
    quiesced: Arc<AtomicUsize>,
    entered: Arc<Notify>,
}

async fn fixture() -> Fixture {
    let manager = Manager::with_config(
        ManagerConfig::default()
            .with_release_queue_workers(1)
            .with_retirement_queue_capacity(1),
    );
    let destroyed = Arc::new(AtomicUsize::new(0));
    let topology = QuiescingTopology::default();
    let quiesced = Arc::clone(&topology.quiesced);
    let entered = Arc::clone(&topology.entered);
    manager
        .register(RegistrationSpec {
            resource: LeasedResource {
                destroyed: Arc::clone(&destroyed),
            },
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology,
            recovery_gate: None,
        })
        .unwrap();
    let row = manager
        .lookup::<LeasedResource>(&ScopeLevel::Global)
        .unwrap();
    let context = ResourceContext::minimal(
        Default::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let guard = manager
        .run_acquire_dispatch(row, &context, &Default::default())
        .await
        .unwrap();
    Fixture {
        manager,
        guard,
        destroyed,
        quiesced,
        entered,
    }
}

#[tokio::test(start_paused = true)]
async fn custom_quiescence_preserves_live_instance_and_cancelled_drain_resumes() {
    let Fixture {
        manager,
        guard,
        destroyed,
        quiesced,
        entered,
    } = fixture().await;
    let mut shutdown = Box::pin(manager.graceful_shutdown(ShutdownConfig::default()));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    entered.notified().await;
    assert_eq!(guard.fetch_add(5, Ordering::SeqCst), 7);
    assert_eq!(destroyed.load(Ordering::SeqCst), 0);
    assert_eq!(quiesced.load(Ordering::SeqCst), 1);
    drop(shutdown);
    let _release_outcome = guard.release().await.unwrap();
    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(quiesced.load(Ordering::SeqCst), 1);
    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test(start_paused = true)]
async fn retirement_coordinators_cannot_starve_external_releases() {
    let manager = Manager::new();
    let destroyed = Arc::new(AtomicUsize::new(0));
    let quiesced = Arc::new(AtomicUsize::new(0));
    let workflow_scope = ScopeLevel::Workflow(nebula_core::WorkflowId::new());

    for scope in [ScopeLevel::Global, workflow_scope.clone()] {
        manager
            .register(RegistrationSpec {
                resource: LeasedResource {
                    destroyed: Arc::clone(&destroyed),
                },
                config: Config,
                scope,
                slot_identity: SlotIdentity::Unbound,
                topology: QuiescingTopology {
                    quiesced: Arc::clone(&quiesced),
                    ..QuiescingTopology::default()
                },
                recovery_gate: None,
            })
            .unwrap();
    }

    let context = ResourceContext::minimal(
        Default::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let global = manager
        .run_acquire_dispatch(
            manager
                .lookup::<LeasedResource>(&ScopeLevel::Global)
                .unwrap(),
            &context,
            &Default::default(),
        )
        .await
        .unwrap();
    let workflow = manager
        .run_acquire_dispatch(
            manager.lookup::<LeasedResource>(&workflow_scope).unwrap(),
            &context,
            &Default::default(),
        )
        .await
        .unwrap();

    let shutdown_config = ShutdownConfig::default()
        .with_drain_timeout(Duration::from_secs(1))
        .with_release_queue_timeout(Duration::from_secs(1));
    let mut shutdown = Box::pin(manager.graceful_shutdown(shutdown_config));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_millis(100), async {
        while quiesced.load(Ordering::SeqCst) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both default-lane retirement coordinators start");

    let (global_release, workflow_release) = tokio::join!(
        tokio::time::timeout(Duration::from_millis(100), global.release()),
        tokio::time::timeout(Duration::from_millis(100), workflow.release()),
    );
    assert_eq!(
        global_release
            .expect("global release is not queued behind retirement")
            .unwrap(),
        crate::ReleaseOutcome::Completed,
    );
    assert_eq!(
        workflow_release
            .expect("workflow release is not queued behind retirement")
            .unwrap(),
        crate::ReleaseOutcome::Completed,
    );

    let report = shutdown.await.unwrap();
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
    assert_eq!(destroyed.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn manager_drop_after_cancelled_drain_never_republishes_retired_rows() {
    let Fixture {
        manager,
        guard,
        quiesced,
        entered,
        ..
    } = fixture().await;
    let mut shutdown = Box::pin(manager.graceful_shutdown(ShutdownConfig::default()));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    entered.notified().await;
    drop(shutdown);
    let queue = Arc::clone(&manager.release_queue);
    drop(manager);
    drop(guard);
    tokio::task::yield_now().await;
    assert_eq!(quiesced.load(Ordering::SeqCst), 1);
    assert_eq!(queue.dropped_count(), 1, "late guard loss is counted once");
}

#[tokio::test(start_paused = true)]
async fn publication_failure_with_live_guard_keeps_release_workers_owned() {
    let Fixture { manager, guard, .. } = fixture().await;
    let supervisor = manager.retirement_supervisor.take_handle().unwrap();
    assert!(supervisor.join_bounded(Duration::ZERO).await.is_err());
    let error = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap_err();
    std::assert_matches!(error, ShutdownError::RetirementSupervisorFailed);
    assert!(manager.release_queue_handle.lock().await.is_some());
    assert_eq!(guard.load(Ordering::SeqCst), 7);
    let _release_outcome = guard.release().await.unwrap();
    assert_eq!(manager.drain_tracker.0.load(Ordering::Acquire), 0);
    assert_eq!(
        manager.release_queue.dropped_count(),
        1,
        "unpublished root abandonment is counted once"
    );
}

#[tokio::test(start_paused = true)]
async fn saturated_publication_uses_original_envelope_and_keeps_late_release_open() {
    let Fixture { manager, guard, .. } = fixture().await;
    let held_capacity = manager.retirement_supervisor.try_reserve().unwrap();
    let started = tokio::time::Instant::now();
    let error = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::from_secs(2))
                .with_release_queue_timeout(Duration::from_secs(3))
                .with_drain_timeout_policy(DrainTimeoutPolicy::Force),
        )
        .await
        .unwrap_err();
    std::assert_matches!(error, ShutdownError::ReleaseQueueTimeout { .. });
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert!(manager.release_queue_handle.lock().await.is_some());
    assert_eq!(manager.retirement_tracker.0.load(Ordering::Acquire), 0);
    drop(held_capacity);
    let _release_outcome = guard.release().await.unwrap();
    assert_eq!(manager.drain_tracker.0.load(Ordering::Acquire), 0);
    assert_eq!(
        manager.release_queue.dropped_count(),
        1,
        "timed-out root abandonment is counted once"
    );
}

#[tokio::test(start_paused = true)]
async fn cancelled_drain_before_publication_resumes_without_second_snapshot() {
    let Fixture {
        manager,
        guard,
        destroyed,
        quiesced,
        entered,
    } = fixture().await;
    let held_capacity = manager.retirement_supervisor.try_reserve().unwrap();
    let mut shutdown = Box::pin(manager.graceful_shutdown(ShutdownConfig::default()));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    drop(shutdown);
    assert_eq!(manager.retirement_tracker.0.load(Ordering::Acquire), 1);
    drop(held_capacity);
    entered.notified().await;
    let _release_outcome = guard.release().await.unwrap();
    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap();
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(quiesced.load(Ordering::SeqCst), 1);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test(start_paused = true)]
async fn accepted_publication_observed_after_deadline_never_duplicates_ownership() {
    let Fixture {
        manager,
        guard,
        destroyed,
        quiesced,
        entered,
    } = fixture().await;
    let mut shutdown = Box::pin(
        manager.graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::from_secs(1))
                .with_release_queue_timeout(Duration::from_secs(1)),
        ),
    );
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    drop(shutdown);
    // The publisher has committed, but the cancelled driver has not observed its result.
    entered.notified().await;
    let _release_outcome = guard.release().await.unwrap();
    tokio::time::advance(Duration::from_secs(3)).await;
    let result = manager.graceful_shutdown(ShutdownConfig::default()).await;
    std::assert_matches!(
        result,
        Ok(_) | Err(ShutdownError::ReleaseQueueTimeout { .. })
    );
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(quiesced.load(Ordering::SeqCst), 1);
    assert_eq!(manager.retirement_tracker.0.load(Ordering::Acquire), 0);
    assert_eq!(manager.drain_tracker.0.load(Ordering::Acquire), 0);
    assert_eq!(manager.release_queue.dropped_count(), 0);
    assert!(manager.release_queue_handle.lock().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn force_confirms_publication_and_preserves_late_custom_lease_destruction() {
    let Fixture {
        manager,
        guard,
        destroyed,
        quiesced,
        ..
    } = fixture().await;
    let report = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::from_secs(1))
                .with_drain_timeout_policy(DrainTimeoutPolicy::Force),
        )
        .await
        .unwrap();
    assert_eq!(report.outstanding_handles_after_drain, 1);
    assert!(!report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
    assert_eq!(quiesced.load(Ordering::SeqCst), 1);
    assert_eq!(destroyed.load(Ordering::SeqCst), 0);
    assert_eq!(guard.load(Ordering::SeqCst), 7);
    let _release_outcome = guard.release().await.unwrap();
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(manager.drain_tracker.0.load(Ordering::Acquire), 0);
    assert_eq!(manager.release_queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn scoped_shared_rows_remain_usable_until_each_final_consumer_releases() {
    let manager = Arc::new(Manager::with_config(
        ManagerConfig::default().with_release_queue_workers(1),
    ));
    let organizations = [nebula_core::OrgId::new(), nebula_core::OrgId::new()];
    let destroyed = [Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0))];
    let mut leases = Vec::new();
    for (organization, destroyed) in organizations.into_iter().zip(&destroyed) {
        manager
            .register(RegistrationSpec {
                resource: LeasedResource {
                    destroyed: Arc::clone(destroyed),
                },
                config: Config,
                scope: ScopeLevel::Organization(organization),
                slot_identity: SlotIdentity::Unbound,
                topology: QuiescingTopology::default(),
                recovery_gate: None,
            })
            .unwrap();
        let context = ResourceContext::minimal(
            nebula_core::Scope {
                org_id: Some(organization),
                ..Default::default()
            },
            tokio_util::sync::CancellationToken::new(),
        );
        let mut consumers = Vec::new();
        for _ in 0..2 {
            let guard = Manager::acquire_any(
                Arc::clone(&manager),
                &LeasedResource::key(),
                &context,
                &Default::default(),
                &SlotIdentity::Unbound,
            )
            .await
            .unwrap();
            consumers.push(*guard.downcast::<ResourceGuard<LeasedResource>>().unwrap());
        }
        leases.push(consumers);
    }
    let mut shutdown = Box::pin(manager.graceful_shutdown(ShutdownConfig::default()));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    super::shutdown::wait_for_tracker_drain(&manager.retirement_tracker, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(destroyed[0].load(Ordering::SeqCst), 0);
    assert_eq!(destroyed[1].load(Ordering::SeqCst), 0);
    assert_eq!(leases[0][0].fetch_add(1, Ordering::SeqCst), 7);
    assert_eq!(leases[0][1].load(Ordering::SeqCst), 8);
    assert_eq!(leases[1][0].load(Ordering::SeqCst), 7);
    let mut first_row = leases.remove(0);
    let _release_outcome = first_row.pop().unwrap().release().await.unwrap();
    assert_eq!(destroyed[0].load(Ordering::SeqCst), 0);
    let _release_outcome = first_row.pop().unwrap().release().await.unwrap();
    assert_eq!(destroyed[0].load(Ordering::SeqCst), 1);
    assert_eq!(destroyed[1].load(Ordering::SeqCst), 0);
    for guard in leases.pop().unwrap() {
        let _release_outcome = guard.release().await.unwrap();
    }
    let report = shutdown.await.unwrap();
    assert_eq!(destroyed[1].load(Ordering::SeqCst), 1);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test(start_paused = true)]
async fn aborted_drain_observes_publisher_failure_after_original_envelope() {
    let Fixture { manager, guard, .. } = fixture().await;
    let held_capacity = manager.retirement_supervisor.try_reserve().unwrap();
    let started = tokio::time::Instant::now();
    let error = manager
        .graceful_shutdown(
            ShutdownConfig::default()
                .with_drain_timeout(Duration::from_secs(2))
                .with_release_queue_timeout(Duration::from_secs(3)),
        )
        .await
        .unwrap_err();
    std::assert_matches!(error, ShutdownError::DrainTimeout { outstanding: 1 });
    tokio::time::advance(Duration::from_secs(4)).await;
    tokio::task::yield_now().await;
    let error = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .unwrap_err();
    std::assert_matches!(error, ShutdownError::ReleaseQueueTimeout { .. });
    assert_eq!(started.elapsed(), Duration::from_secs(6));
    assert_eq!(manager.retirement_tracker.0.load(Ordering::Acquire), 0);
    assert!(manager.release_queue_handle.lock().await.is_some());
    drop(held_capacity);
    let _release_outcome = guard.release().await.unwrap();
    assert_eq!(manager.drain_tracker.0.load(Ordering::Acquire), 0);
    assert_eq!(
        manager.release_queue.dropped_count(),
        1,
        "resumed failure does not count the same root twice"
    );
}

#[tokio::test(start_paused = true)]
async fn zero_drain_budget_still_observes_an_already_released_lease() {
    let Fixture {
        manager,
        guard,
        destroyed,
        ..
    } = fixture().await;
    let _release_outcome = guard.release().await.unwrap();
    let report = manager
        .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::ZERO))
        .await
        .unwrap();
    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
}

#[derive(Clone)]
struct ParentResource {
    child: Arc<std::sync::Mutex<Option<ResourceGuard<LeasedResource>>>>,
    destroyed: Arc<AtomicUsize>,
}
crate::no_credential_slots!(ParentResource);

#[async_trait::async_trait]
impl Provider for ParentResource {
    type Config = Config;
    type Instance = ResourceGuard<LeasedResource>;
    type Topology = crate::Resident<Self>;
    fn key() -> nebula_core::ResourceKey {
        nebula_core::resource_key!("shutdown-session-parent")
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("shutdown-session-parent"),
            "",
        )
    }
    async fn create(&self, _: &Config, _: &ResourceContext) -> Result<Self::Instance, Error> {
        self.child
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| Error::permanent("test parent requires an installed child"))
    }
    async fn destroy(&self, child: Self::Instance, _: TeardownCx) -> Result<(), Error> {
        drop(child);
        self.destroyed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl crate::topology::ResidentProvider for ParentResource {
    fn is_alive_sync(&self, _: &Self::Instance) -> bool {
        true
    }
}

#[tokio::test(start_paused = true)]
async fn external_parent_and_child_stay_live_after_both_roots_retire() {
    let Fixture {
        manager,
        guard: external_child,
        destroyed: child_destroyed,
        ..
    } = fixture().await;
    let context = ResourceContext::minimal(
        Default::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let child_row = manager
        .lookup::<LeasedResource>(&ScopeLevel::Global)
        .unwrap();
    let internal_child = manager
        .run_acquire_dispatch(child_row, &context, &Default::default())
        .await
        .unwrap();
    let parent_destroyed = Arc::new(AtomicUsize::new(0));
    manager
        .register(RegistrationSpec {
            resource: ParentResource {
                child: Arc::new(std::sync::Mutex::new(Some(internal_child))),
                destroyed: Arc::clone(&parent_destroyed),
            },
            config: Config,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: crate::Resident::new(Default::default()),
            recovery_gate: None,
        })
        .unwrap();
    let parent = manager
        .acquire_resident::<ParentResource>(&context, &Default::default())
        .await
        .unwrap();
    let mut shutdown = Box::pin(manager.graceful_shutdown(ShutdownConfig::default()));
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    super::shutdown::wait_for_tracker_drain(&manager.retirement_tracker, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(parent.fetch_add(3, Ordering::SeqCst), 7);
    assert_eq!(external_child.load(Ordering::SeqCst), 10);
    assert_eq!(parent_destroyed.load(Ordering::SeqCst), 0);
    assert_eq!(child_destroyed.load(Ordering::SeqCst), 0);
    let _parent_outcome = parent.release().await.unwrap();
    assert_eq!(parent_destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(child_destroyed.load(Ordering::SeqCst), 0);
    let _child_outcome = external_child.release().await.unwrap();
    let report = shutdown.await.unwrap();
    assert_eq!(child_destroyed.load(Ordering::SeqCst), 1);
    assert_eq!(report.outstanding_handles_after_drain, 0);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
}
