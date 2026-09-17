use std::{
    any::{Any, TypeId},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::Duration,
};

use tokio::sync::Notify;

use super::*;
use crate::{
    AcquireOptions, ResourceContext, TopologyTag,
    runtime::acquire_loop::{AcceptedSlotHook, SlotHookAdmission, SlotHookSettlement},
    topology::{AdmissionPhase, Load, Unavailable},
};

struct DeferredRetirementHandle {
    maintenance_entered: Notify,
    maintenance_release: Notify,
    entered: Notify,
    release: Notify,
    completed: AtomicUsize,
    waits_for_maintenance: bool,
    waits_for_release: bool,
    maintenance_fails: bool,
    terminal_fails: bool,
}

#[async_trait::async_trait]
impl ManagedHandle for DeferredRetirementHandle {
    fn resource_key(&self) -> ResourceKey {
        ResourceKey::new("test.deferred-retirement").expect("valid static resource key")
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn managed_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }

    fn set_phase(&self, _phase: crate::state::ResourcePhase) {}

    fn set_failed(&self, _kind: crate::ErrorKind, _reason: &str) {}

    fn phase(&self) -> crate::state::ResourcePhase {
        crate::state::ResourcePhase::Ready
    }

    fn begin_close(&self) {}

    fn abort_maintenance(&self) {}

    async fn close_retained(self: Arc<Self>) -> Result<(), Error> {
        self.entered.notify_one();
        if self.waits_for_release {
            self.release.notified().await;
        }
        self.completed.fetch_add(1, Ordering::SeqCst);
        if self.terminal_fails {
            Err(Error::transient("provider detail stays out of events"))
        } else {
            Ok(())
        }
    }

    async fn join_maintenance(&self) -> Result<(), Error> {
        self.maintenance_entered.notify_one();
        if self.waits_for_maintenance {
            self.maintenance_release.notified().await;
        }
        if self.maintenance_fails {
            Err(Error::permanent("maintenance detail stays out of events"))
        } else {
            Ok(())
        }
    }

    fn topology_tag(&self) -> TopologyTag {
        TopologyTag::Resident
    }

    fn taint(&self) {}

    fn bump_revoke_epoch(&self) {}

    fn accepts_credential_slot_name(&self, _slot: &str) -> bool {
        true
    }

    fn install_credential_slot(
        &self,
        _slot: &str,
        _guard: nebula_credential::ErasedCredentialGuard,
    ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
        unreachable!("retirement fake never installs credential slots")
    }

    fn revoke_credential_slot(
        &self,
        _slot: &str,
    ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
        unreachable!("retirement fake never revokes credential slots")
    }

    fn submit_on_refresh(
        self: Arc<Self>,
        _slot: &str,
        _timeout: Duration,
        _settlement: SlotHookSettlement,
        _admission: SlotHookAdmission,
    ) -> Result<AcceptedSlotHook, Error> {
        unreachable!("retirement fake never dispatches credential hooks")
    }

    fn submit_on_revoke(
        self: Arc<Self>,
        _slot: &str,
        _timeout: Duration,
        _settlement: SlotHookSettlement,
        _admission: SlotHookAdmission,
    ) -> Result<AcceptedSlotHook, Error> {
        unreachable!("retirement fake never dispatches credential hooks")
    }

    async fn wait_for_in_flight_drain(&self, _timeout: Duration) -> Result<(), u64> {
        Ok(())
    }

    fn admission_phase(&self) -> AdmissionPhase {
        AdmissionPhase::Ready
    }

    fn try_reserve_gate(&self) -> Result<(), Unavailable> {
        Ok(())
    }

    fn admission_load(&self) -> Option<Load> {
        None
    }

    async fn acquire(
        self: Arc<Self>,
        _manager: Arc<crate::Manager>,
        _context: ResourceContext,
        _options: AcquireOptions,
    ) -> Result<Box<dyn Any + Send + Sync>, Error> {
        Err(Error::permanent("test handle does not support acquire"))
    }
}

#[tokio::test]
async fn supervisor_handle_observes_completion_without_a_parallel_receipt() {
    let handle = tokio::spawn(async { None });
    let owner = RetirementSupervisorHandle {
        handle: Some(handle),
    };
    let result = owner.join_bounded(Duration::from_secs(1)).await;
    assert!(matches!(result, Ok(None)));
}

#[tokio::test]
async fn supervisor_handle_observes_failure_after_the_task_was_scheduled() {
    let handle = tokio::spawn(async {
        tokio::task::yield_now().await;
        panic!("injected supervisor failure");
    });
    let owner = RetirementSupervisorHandle {
        handle: Some(handle),
    };
    let result = owner.join_bounded(Duration::from_secs(1)).await;
    assert!(matches!(result, Err(RetirementSupervisorJoinError::Failed)));
}

#[tokio::test]
async fn prior_non_shutdown_failures_do_not_fail_healthy_graceful_shutdown() {
    let (release_queue, workers) = ReleaseQueue::new(1);
    let release_queue = Arc::new(release_queue);
    let supervisor = RetirementSupervisor::new(Arc::clone(&release_queue), 1, 4);
    let tracker = Arc::new((AtomicU64::new(0), Notify::new()));
    let event_bus = Arc::new(EventBus::new(8));
    let mut events = event_bus.subscribe();

    for (origin, terminal_fails) in [
        (RetirementOrigin::Removal, true),
        (RetirementOrigin::Replacement, true),
        (RetirementOrigin::Shutdown, false),
    ] {
        let managed = Arc::new(DeferredRetirementHandle {
            maintenance_entered: Notify::new(),
            maintenance_release: Notify::new(),
            entered: Notify::new(),
            release: Notify::new(),
            completed: AtomicUsize::new(0),
            waits_for_maintenance: false,
            waits_for_release: false,
            maintenance_fails: false,
            terminal_fails,
        });
        supervisor
            .try_reserve()
            .expect("retirement queue has capacity")
            .commit(PendingRetirement::new(
                managed,
                RetirementSettlement::new(Arc::clone(&tracker)),
                Arc::clone(&event_bus),
                None,
                origin,
            ));
    }

    supervisor.seal().await.expect("supervisor accepts seal");
    let joined = supervisor
        .take_handle()
        .expect("supervisor handle remains owned")
        .join_bounded(Duration::from_secs(1))
        .await;
    let Ok(failure) = joined else {
        panic!("supervisor must join within budget")
    };
    assert!(
        failure.is_none(),
        "historical removal and replacement failures must not become the shutdown aggregate"
    );
    std::assert_matches!(
        events.try_recv(),
        Some(ResourceEvent::ResourceTeardownFailed {
            origin: RetirementOrigin::Removal,
            stage: RetirementFailureStage::TerminalCleanup,
            kind: crate::ErrorKind::Transient,
            ..
        })
    );
    std::assert_matches!(
        events.try_recv(),
        Some(ResourceEvent::ResourceTeardownFailed {
            origin: RetirementOrigin::Replacement,
            stage: RetirementFailureStage::TerminalCleanup,
            kind: crate::ErrorKind::Transient,
            ..
        })
    );
    assert!(events.try_recv().is_none());
    assert_eq!(tracker.0.load(Ordering::Acquire), 0);

    release_queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test]
async fn each_failing_retirement_stage_emits_its_own_redacted_event() {
    let managed = Arc::new(DeferredRetirementHandle {
        maintenance_entered: Notify::new(),
        maintenance_release: Notify::new(),
        entered: Notify::new(),
        release: Notify::new(),
        completed: AtomicUsize::new(0),
        waits_for_maintenance: false,
        waits_for_release: false,
        maintenance_fails: true,
        terminal_fails: true,
    });
    let tracker = Arc::new((AtomicU64::new(0), Notify::new()));
    let event_bus = Arc::new(EventBus::new(8));
    let mut events = event_bus.subscribe();
    let retirement = PendingRetirement::new(
        Arc::<DeferredRetirementHandle>::clone(&managed),
        RetirementSettlement::new(Arc::clone(&tracker)),
        event_bus,
        None,
        RetirementOrigin::Removal,
    );
    let (release_queue, workers) = ReleaseQueue::new(1);
    let release_queue = Arc::new(release_queue);

    let completion = retirement.close(Arc::clone(&release_queue)).await;
    let Some(failure) = completion else {
        panic!("maintenance failure must remain the aggregate result");
    };
    assert_eq!(failure.source.kind(), &crate::ErrorKind::Permanent);

    let first = events.try_recv().expect("maintenance failure is observed");
    let second = events.try_recv().expect("terminal failure is observed");
    std::assert_matches!(
        first,
        ResourceEvent::ResourceTeardownFailed {
            origin: RetirementOrigin::Removal,
            stage: RetirementFailureStage::Maintenance,
            kind: crate::ErrorKind::Permanent,
            ..
        }
    );
    std::assert_matches!(
        second,
        ResourceEvent::ResourceTeardownFailed {
            origin: RetirementOrigin::Removal,
            stage: RetirementFailureStage::TerminalCleanup,
            kind: crate::ErrorKind::Transient,
            ..
        }
    );
    assert_eq!(tracker.0.load(Ordering::Acquire), 0);

    release_queue.close();
    ReleaseQueue::shutdown(workers).await;
}

fn assert_one_cancelled_terminal_event(events: &mut nebula_eventbus::Subscriber<ResourceEvent>) {
    std::assert_matches!(
        events.try_recv(),
        Some(ResourceEvent::ResourceTeardownFailed {
            origin: RetirementOrigin::Shutdown,
            stage: RetirementFailureStage::TerminalCleanup,
            kind: crate::ErrorKind::Cancelled,
            ..
        })
    );
    assert!(
        events.try_recv().is_none(),
        "terminal abandonment must emit exactly once"
    );
}

#[tokio::test]
async fn abort_during_maintenance_join_emits_one_failure_and_releases_tracker() {
    let managed = Arc::new(DeferredRetirementHandle {
        maintenance_entered: Notify::new(),
        maintenance_release: Notify::new(),
        entered: Notify::new(),
        release: Notify::new(),
        completed: AtomicUsize::new(0),
        waits_for_maintenance: true,
        waits_for_release: false,
        maintenance_fails: false,
        terminal_fails: false,
    });
    let tracker = Arc::new((AtomicU64::new(0), Notify::new()));
    let event_bus = Arc::new(EventBus::new(8));
    let mut events = event_bus.subscribe();
    let retirement = PendingRetirement::new(
        Arc::<DeferredRetirementHandle>::clone(&managed),
        RetirementSettlement::new(Arc::clone(&tracker)),
        event_bus,
        None,
        RetirementOrigin::Shutdown,
    );
    let (release_queue, workers) = ReleaseQueue::new(1);
    let release_queue = Arc::new(release_queue);
    let close_queue = Arc::clone(&release_queue);
    let close_task = tokio::spawn(async move { retirement.close(close_queue).await });
    managed.maintenance_entered.notified().await;

    close_task.abort();
    assert!(
        close_task
            .await
            .expect_err("close task was aborted")
            .is_cancelled(),
        "maintenance-join cancellation must acknowledge task abort"
    );
    assert_eq!(managed.completed.load(Ordering::SeqCst), 0);
    assert_eq!(tracker.0.load(Ordering::Acquire), 0);
    assert_one_cancelled_terminal_event(&mut events);
    assert_eq!(release_queue.dropped_count(), 0);

    release_queue.close();
    ReleaseQueue::shutdown(workers).await;
}

#[tokio::test(start_paused = true)]
async fn buffered_coordinator_abort_emits_one_failure_and_releases_tracker() {
    let (release_queue, workers) = ReleaseQueue::new(1);
    let release_queue = Arc::new(release_queue);
    let blocker_entered = Arc::new(Notify::new());
    let entered = Arc::clone(&blocker_entered);
    let _blocker_submission = release_queue
        .submit_coordinator(move || {
            Box::pin(async move {
                entered.notify_one();
                std::future::pending::<Result<(), Error>>().await
            })
        })
        .expect("open coordinator lane accepts blocker");
    blocker_entered.notified().await;

    let managed = Arc::new(DeferredRetirementHandle {
        maintenance_entered: Notify::new(),
        maintenance_release: Notify::new(),
        entered: Notify::new(),
        release: Notify::new(),
        completed: AtomicUsize::new(0),
        waits_for_maintenance: false,
        waits_for_release: false,
        maintenance_fails: false,
        terminal_fails: false,
    });
    let tracker = Arc::new((AtomicU64::new(0), Notify::new()));
    let event_bus = Arc::new(EventBus::new(8));
    let mut events = event_bus.subscribe();
    let retirement = PendingRetirement::new(
        Arc::<DeferredRetirementHandle>::clone(&managed),
        RetirementSettlement::new(Arc::clone(&tracker)),
        event_bus,
        None,
        RetirementOrigin::Shutdown,
    );
    let close_queue = Arc::clone(&release_queue);
    let close_task = tokio::spawn(async move { retirement.close(close_queue).await });
    tokio::task::yield_now().await;

    let _shutdown_error = ReleaseQueue::shutdown_bounded(workers, Duration::ZERO)
        .await
        .expect_err("zero budget aborts running and buffered coordinators");
    let completion = close_task.await.expect("retirement task does not panic");
    let Some(failure) = completion else {
        panic!("aborted buffered retirement must report cancellation");
    };
    assert_eq!(failure.source.kind(), &crate::ErrorKind::Cancelled);
    assert_eq!(managed.completed.load(Ordering::SeqCst), 0);
    assert_eq!(tracker.0.load(Ordering::Acquire), 0);
    assert_one_cancelled_terminal_event(&mut events);
    assert_eq!(release_queue.dropped_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn running_coordinator_abort_emits_one_failure_and_releases_tracker() {
    let managed = Arc::new(DeferredRetirementHandle {
        maintenance_entered: Notify::new(),
        maintenance_release: Notify::new(),
        entered: Notify::new(),
        release: Notify::new(),
        completed: AtomicUsize::new(0),
        waits_for_maintenance: false,
        waits_for_release: true,
        maintenance_fails: false,
        terminal_fails: false,
    });
    let tracker = Arc::new((AtomicU64::new(0), Notify::new()));
    let event_bus = Arc::new(EventBus::new(8));
    let mut events = event_bus.subscribe();
    let retirement = PendingRetirement::new(
        Arc::<DeferredRetirementHandle>::clone(&managed),
        RetirementSettlement::new(Arc::clone(&tracker)),
        event_bus,
        None,
        RetirementOrigin::Shutdown,
    );
    let (release_queue, workers) = ReleaseQueue::new(1);
    let release_queue = Arc::new(release_queue);
    let close_queue = Arc::clone(&release_queue);
    let close_task = tokio::spawn(async move { retirement.close(close_queue).await });
    managed.entered.notified().await;

    let _shutdown_error = ReleaseQueue::shutdown_bounded(workers, Duration::ZERO)
        .await
        .expect_err("zero budget aborts the running coordinator");
    let completion = close_task.await.expect("retirement task does not panic");
    let Some(failure) = completion else {
        panic!("aborted running retirement must report cancellation");
    };
    assert_eq!(failure.source.kind(), &crate::ErrorKind::Cancelled);
    assert_eq!(managed.completed.load(Ordering::SeqCst), 0);
    assert_eq!(tracker.0.load(Ordering::Acquire), 0);
    assert_one_cancelled_terminal_event(&mut events);
    assert_eq!(release_queue.dropped_count(), 0);
}
