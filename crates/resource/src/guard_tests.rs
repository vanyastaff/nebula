use super::*;
use crate::{
    AcquireOptions, Manager, RegistrationSpec,
    dedup::SlotIdentity,
    topology::{InstanceMetrics, PoolProvider, Pooled, RecycleDecision},
};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[derive(Clone)]
struct DummyResource {
    released: Arc<AtomicU32>,
    drops: Arc<AtomicU32>,
    tainted: Arc<AtomicBool>,
    fault: &'static str,
}

// Deliberately not Clone: the guard must borrow the actual owning entry.
struct Payload {
    value: u32,
    drops: Arc<AtomicU32>,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl DummyResource {
    fn new(fault: &'static str) -> Self {
        Self {
            released: Arc::new(AtomicU32::new(0)),
            drops: Arc::new(AtomicU32::new(0)),
            tainted: Arc::new(AtomicBool::new(false)),
            fault,
        }
    }
}

#[async_trait::async_trait]
impl Provider for DummyResource {
    type Config = ();
    type Instance = Payload;
    type Topology = Pooled<Self>;

    fn metadata() -> crate::ResourceMetadataDraft {
        crate::ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("DummyResource"), "")
    }

    fn key() -> ResourceKey {
        nebula_core::resource_key!("guard-test")
    }

    async fn create(&self, (): &(), _: &ResourceContext) -> Result<Payload, crate::Error> {
        Ok(Payload {
            value: 42,
            drops: Arc::clone(&self.drops),
        })
    }

    async fn destroy(
        &self,
        instance: Payload,
        context: crate::TeardownCx,
    ) -> Result<(), crate::Error> {
        assert_eq!(instance.value, 42, "destroy receives the original instance");
        self.tainted.store(
            matches!(context.reason, crate::TeardownReason::Revoked),
            Ordering::SeqCst,
        );
        self.released.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

crate::no_credential_slots!(DummyResource);

impl PoolProvider for DummyResource {
    async fn recycle(
        &self,
        _: &Payload,
        _: &InstanceMetrics,
    ) -> Result<RecycleDecision, crate::Error> {
        match self.fault {
            "panic" => panic!("intentional author recycle panic"),
            "timeout" => std::future::pending().await,
            _ => Ok(RecycleDecision::Drop),
        }
    }
}

fn watchdog_test_ctx() -> ResourceContext {
    ResourceContext::minimal(
        nebula_core::scope::Scope {
            execution_id: Some(nebula_core::ExecutionId::new()),
            workflow_id: Some(nebula_core::WorkflowId::new()),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
    )
}

async fn acquire(resource: DummyResource) -> (Manager, ResourceGuard<DummyResource>) {
    let manager = Manager::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: (),
            scope: nebula_core::scope::ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Pooled::new(
                crate::topology::pooled::config::Config {
                    min_size: 0,
                    max_size: 1,
                    idle_timeout: None,
                    max_lifetime: None,
                    ..Default::default()
                },
                0,
            ),
            recovery_gate: None,
        })
        .unwrap();
    let guard = manager
        .acquire_pooled::<DummyResource>(&watchdog_test_ctx(), &AcquireOptions::default())
        .await
        .unwrap();
    (manager, guard)
}

#[tokio::test]
async fn manager_guard_borrows_non_clone_entry_and_releases_once() {
    use nebula_core::{Guard, TypedGuard};
    let resource = DummyResource::new("");
    let (_manager, guard) = acquire(resource.clone()).await;
    assert_eq!(guard.value, 42);
    assert_eq!(guard.as_inner().value, 42);
    assert_eq!(guard.guard_kind(), "resource");
    assert_eq!(guard.resource_key(), &DummyResource::key());
    assert_eq!(guard.topology_tag(), TopologyTag::Pool);
    assert_eq!(guard.generation(), 0);
    assert!(guard.acquired_at().elapsed() < Duration::from_secs(1));
    assert!(guard.hold_duration() < Duration::from_millis(100));
    assert_eq!(resource.drops.load(Ordering::SeqCst), 0);
    assert_eq!(guard.release().await.unwrap(), ReleaseOutcome::Completed);
    assert_eq!(resource.released.load(Ordering::SeqCst), 1);
    assert_eq!(resource.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guard_drop_retains_reservations_until_cleanup_and_drops_once() {
    let resource = DummyResource::new("");
    let (manager, guard) = acquire(resource.clone()).await;
    drop(guard);
    let report = manager
        .graceful_shutdown(crate::ShutdownConfig::default())
        .await
        .unwrap();
    assert!(report.release_queue_drained);
    assert_eq!(resource.released.load(Ordering::SeqCst), 1);
    assert_eq!(resource.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn taint_bypasses_recycle_and_destroys_as_revoked() {
    let resource = DummyResource::new("panic");
    let (_manager, mut guard) = acquire(resource.clone()).await;
    guard.taint();
    assert_eq!(guard.release().await.unwrap(), ReleaseOutcome::Completed);
    assert!(resource.tainted.load(Ordering::SeqCst));
    assert_eq!(resource.released.load(Ordering::SeqCst), 1);
    assert_eq!(resource.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn manager_drop_rejects_late_release_but_settles_ownership() {
    let resource = DummyResource::new("");
    let (manager, guard) = acquire(resource.clone()).await;
    let queue = Arc::clone(&guard.managed.release_queue);
    let (global, row) = guard.drain_counters.clone().unwrap();
    drop(manager);
    let error = guard
        .release()
        .await
        .expect_err("manager Drop closes admission to cleanup");
    assert_eq!(error.kind(), &crate::ErrorKind::Cancelled);
    assert_eq!(global.0.load(Ordering::Acquire), 0);
    assert_eq!(row.0.load(Ordering::Acquire), 0);
    assert_eq!(queue.dropped_count(), 1);
    assert_eq!(resource.released.load(Ordering::SeqCst), 0);
    assert_eq!(resource.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn queued_guard_hook_faults_remain_visible_to_loss_accounting() {
    for fault in ["panic", "timeout"] {
        let resource = DummyResource::new(fault);
        let (manager, guard) = acquire(resource.clone()).await;
        assert!(guard.release().await.is_err());
        let report = manager
            .graceful_shutdown(crate::ShutdownConfig::default())
            .await
            .unwrap();
        assert_eq!(
            report.dropped_release_tasks, 1,
            "lost {fault} teardown counts once"
        );
        assert_eq!(resource.drops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn panicking_cleanup_returns_capacity_after_settlement() {
    let resource = DummyResource::new("panic");
    let (manager, guard) = acquire(resource).await;
    assert!(guard.release().await.is_err());
    let next = manager
        .acquire_pooled::<DummyResource>(&watchdog_test_ctx(), &AcquireOptions::default())
        .await
        .expect("the failed cleanup must release the sole permit");
    assert_eq!(next.value, 42);
    drop(next);
}

#[tokio::test]
async fn hold_watchdog_task_ends_with_the_lease() {
    // A released lease must not leave its watchdog parked for the full
    // deadline: live watchdog tasks are bounded by live leases.
    let bus = Arc::new(EventBus::<ResourceEvent>::new(256));
    let ctx = watchdog_test_ctx();
    let (_manager, guard) = acquire(DummyResource::new("")).await;
    let held = guard.with_event_bus(Arc::clone(&bus)).with_hold_watchdog(
        Some(Duration::from_hours(1)),
        &ctx,
        None,
    );
    let watchdog = held
        .hold_watchdog
        .as_ref()
        .expect("an armed watchdog is owned by the guard")
        .0
        .clone();
    assert!(!watchdog.is_finished());

    drop(held);
    for _ in 0..100 {
        if watchdog.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        watchdog.is_finished(),
        "dropping the lease must abort its watchdog task"
    );
}

#[tokio::test(start_paused = true)]
async fn hold_watchdog_emits_when_lease_overruns_deadline() {
    let bus = Arc::new(EventBus::<ResourceEvent>::new(256));
    let mut events = bus.subscribe();
    let ctx = watchdog_test_ctx();
    let (_manager, guard) = acquire(DummyResource::new("")).await;
    let _held = guard.with_event_bus(Arc::clone(&bus)).with_hold_watchdog(
        Some(Duration::from_secs(1)),
        &ctx,
        None,
    );
    let event = events
        .recv()
        .await
        .expect("watchdog emits the deadline event");
    match event {
        ResourceEvent::HoldDeadlineExceeded {
            key,
            deadline,
            execution_id,
            workflow_id,
            ..
        } => {
            assert_eq!(key, DummyResource::key());
            assert_eq!(deadline, Duration::from_secs(1));
            assert_eq!(execution_id, ctx.execution_id());
            assert_eq!(workflow_id, ctx.scope().workflow_id);
        },
        other => panic!("expected HoldDeadlineExceeded, got {other:?}"),
    }
}
