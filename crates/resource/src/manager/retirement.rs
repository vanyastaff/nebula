//! Bounded, manager-owned orchestration for resource-row retirement.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::{StreamExt as _, stream::FuturesUnordered};
use nebula_core::ResourceKey;
use nebula_credential::SecretFreeMessage;
use nebula_eventbus::EventBus;
use tokio::sync::mpsc;

use crate::{
    error::Error,
    events::{ResourceEvent, RetirementFailureStage, RetirementOrigin},
    metrics::ResourceOpsMetrics,
    registry::ManagedHandle,
    release_queue::{ReleaseQueue, SubmissionOutcome},
    runtime::managed::LeaseAccountingPoisoned,
};

use super::shutdown::RetirementSettlement;

/// Runs bounded row futures in one manager-owned supervisor task.
pub(super) struct RetirementSupervisor {
    commands: mpsc::Sender<RetirementCommand>,
    handle: Mutex<Option<tokio::task::JoinHandle<Option<RetirementFailure>>>>,
}

impl RetirementSupervisor {
    pub(super) fn new(
        release_queue: Arc<ReleaseQueue>,
        concurrency: usize,
        queue_capacity: usize,
    ) -> Self {
        let (commands, receiver) = mpsc::channel(queue_capacity.max(1));
        let handle = tokio::spawn(run_supervisor(receiver, release_queue, concurrency.max(1)));
        Self {
            commands,
            handle: Mutex::new(Some(handle)),
        }
    }

    pub(super) fn try_reserve(&self) -> Result<RetirementPermit, Error> {
        self.commands
            .clone()
            .try_reserve_owned()
            .map(|permit| RetirementPermit { permit })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    Error::backpressure("resource retirement queue is saturated")
                },
                mpsc::error::TrySendError::Closed(_) => Error::cancelled(),
            })
    }

    pub(super) async fn publish_batch_bounded(
        &self,
        retirements: Vec<PendingRetirement>,
        timeout: std::time::Duration,
    ) -> Result<(), PublishBatchError> {
        match tokio::time::timeout(timeout, self.commands.clone().reserve_owned()).await {
            Ok(Ok(permit)) => {
                RetirementPermit { permit }.commit_batch(retirements);
                Ok(())
            },
            Ok(Err(_)) => Err(PublishBatchError::Closed(retirements)),
            Err(_) => Err(PublishBatchError::TimedOut(retirements)),
        }
    }

    pub(super) async fn seal(&self) -> Result<(), Error> {
        self.commands
            .send(RetirementCommand::Seal)
            .await
            .map_err(|_| Error::cancelled())
    }

    pub(super) fn take_handle(&self) -> Option<RetirementSupervisorHandle> {
        self.handle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .map(|handle| RetirementSupervisorHandle {
                handle: Some(handle),
            })
    }
}

pub(super) enum PublishBatchError {
    TimedOut(Vec<PendingRetirement>),
    Closed(Vec<PendingRetirement>),
}

impl Drop for RetirementSupervisor {
    fn drop(&mut self) {
        if let Some(handle) = self
            .handle
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            handle.abort();
        }
    }
}

pub(super) struct RetirementPermit {
    permit: mpsc::OwnedPermit<RetirementCommand>,
}

impl RetirementPermit {
    pub(super) fn commit(self, retirement: PendingRetirement) {
        self.permit.send(RetirementCommand::Retire(retirement));
    }

    pub(super) fn commit_batch(self, retirements: Vec<PendingRetirement>) {
        self.permit
            .send(RetirementCommand::RetireBatch(retirements));
    }
}

/// Owns the supervisor handle across cancellation and acknowledges timeout aborts.
pub(super) struct RetirementSupervisorHandle {
    handle: Option<tokio::task::JoinHandle<Option<RetirementFailure>>>,
}

pub(super) enum RetirementSupervisorJoinError {
    TimedOut,
    Failed,
}

impl RetirementSupervisorHandle {
    pub(super) async fn join_bounded(
        mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<RetirementFailure>, RetirementSupervisorJoinError> {
        let Some(handle) = self.handle.as_mut() else {
            return Err(RetirementSupervisorJoinError::Failed);
        };
        match tokio::time::timeout(timeout, &mut *handle).await {
            Ok(Ok(failure)) => {
                self.handle.take();
                Ok(failure)
            },
            Ok(Err(_)) => {
                self.handle.take();
                Err(RetirementSupervisorJoinError::Failed)
            },
            Err(_) => {
                handle.abort();
                let _ = (&mut *handle).await;
                self.handle.take();
                Err(RetirementSupervisorJoinError::TimedOut)
            },
        }
    }
}

impl Drop for RetirementSupervisorHandle {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }
}

enum RetirementCommand {
    Retire(PendingRetirement),
    RetireBatch(Vec<PendingRetirement>),
    Seal,
}

type RetirementFuture = Pin<Box<dyn Future<Output = Option<RetirementFailure>> + Send + 'static>>;

fn record_retirement_completion(
    first_failure: &mut Option<RetirementFailure>,
    failure: Option<RetirementFailure>,
) {
    if first_failure.is_none()
        && failure
            .as_ref()
            .is_some_and(|failure| failure.origin == RetirementOrigin::Shutdown)
    {
        *first_failure = failure;
    }
}

async fn drain_batch(
    retirements: Vec<PendingRetirement>,
    active: &mut FuturesUnordered<RetirementFuture>,
    release_queue: &Arc<ReleaseQueue>,
    concurrency: usize,
    first_failure: &mut Option<RetirementFailure>,
) {
    let mut pending = VecDeque::from(retirements);
    loop {
        while active.len() < concurrency
            && let Some(retirement) = pending.pop_front()
        {
            active.push(Box::pin(retirement.close(Arc::clone(release_queue))));
        }
        if pending.is_empty() && active.is_empty() {
            return;
        }
        if let Some(completion) = active.next().await {
            record_retirement_completion(first_failure, completion);
        }
    }
}

async fn run_supervisor(
    mut commands: mpsc::Receiver<RetirementCommand>,
    release_queue: Arc<ReleaseQueue>,
    concurrency: usize,
) -> Option<RetirementFailure> {
    let mut active: FuturesUnordered<RetirementFuture> = FuturesUnordered::new();
    let mut first_failure = None;

    loop {
        while active.len() < concurrency {
            match commands.try_recv() {
                Ok(RetirementCommand::Retire(owner)) => {
                    active.push(Box::pin(owner.close(Arc::clone(&release_queue))));
                },
                Ok(RetirementCommand::RetireBatch(retirements)) => {
                    drain_batch(
                        retirements,
                        &mut active,
                        &release_queue,
                        concurrency,
                        &mut first_failure,
                    )
                    .await;
                },
                Ok(RetirementCommand::Seal) => {
                    while let Some(completion) = active.next().await {
                        record_retirement_completion(&mut first_failure, completion);
                    }
                    return first_failure;
                },
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => return first_failure,
            }
        }

        tokio::select! {
            command = commands.recv(), if active.len() < concurrency => match command {
                Some(RetirementCommand::Retire(owner)) => {
                    active.push(Box::pin(owner.close(Arc::clone(&release_queue))));
                },
                Some(RetirementCommand::RetireBatch(retirements)) => {
                    drain_batch(
                        retirements,
                        &mut active,
                        &release_queue,
                        concurrency,
                        &mut first_failure,
                    )
                    .await;
                },
                Some(RetirementCommand::Seal) => {
                    while let Some(completion) = active.next().await {
                        record_retirement_completion(&mut first_failure, completion);
                    }
                    return first_failure;
                },
                None => return first_failure,
            },
            Some(completion) = active.next(), if !active.is_empty() => {
                record_retirement_completion(&mut first_failure, completion);
            },
        }
    }
}

#[derive(Debug)]
#[must_use]
pub(super) struct RetirementFailure {
    pub(super) key: ResourceKey,
    pub(super) origin: RetirementOrigin,
    pub(super) source: Error,
}

/// Fenced row ownership held before bounded supervisor admission.
pub(super) struct PendingRetirement {
    managed: Arc<dyn ManagedHandle>,
    terminal: Option<RetirementTerminalSettlement>,
    observer: Arc<RetirementObserver>,
}

#[derive(Clone)]
struct RetirementObserver {
    event_bus: Arc<EventBus<ResourceEvent>>,
    metrics: Option<ResourceOpsMetrics>,
    origin: RetirementOrigin,
}

impl RetirementObserver {
    fn observe(&self, key: &ResourceKey, default_stage: RetirementFailureStage, error: &Error) {
        let accounting_is_poisoned = match std::error::Error::source(error) {
            Some(source) => source.is::<LeaseAccountingPoisoned>(),
            None => false,
        };
        let stage = if accounting_is_poisoned {
            RetirementFailureStage::LeaseAccounting
        } else {
            default_stage
        };
        let message = match stage {
            RetirementFailureStage::Maintenance => {
                SecretFreeMessage::new("resource maintenance close failed")
            },
            RetirementFailureStage::TerminalCleanup => {
                SecretFreeMessage::new("resource terminal cleanup failed")
            },
            RetirementFailureStage::LeaseAccounting => SecretFreeMessage::new(
                "retained lease accounting poisoned; process restart required",
            ),
        };
        tracing::error!(
            resource.key = %key,
            retirement.origin = ?self.origin,
            retirement.stage = ?stage,
            error.kind = ?error.kind(),
            "resource retirement stage failed"
        );
        if let Some(metrics) = &self.metrics {
            metrics.record_release_error();
        }
        let _ = self.event_bus.emit(ResourceEvent::ResourceTeardownFailed {
            key: key.clone(),
            origin: self.origin,
            stage,
            kind: error.kind().clone(),
            message,
        });
    }
}

/// Queue-owned terminal observation and tracker acknowledgement.
///
/// This guard exists before retirement reaches any await or queue boundary.
/// Dropping an unsettled retirement therefore records one cancellation whether
/// ownership was lost while joining maintenance, buffered in a lane, running
/// on a worker, or unwinding from the coordinator future.
struct RetirementTerminalSettlement {
    key: ResourceKey,
    observer: Arc<RetirementObserver>,
    _tracker: RetirementSettlement,
    is_pending: bool,
}

impl RetirementTerminalSettlement {
    fn new(
        key: ResourceKey,
        observer: Arc<RetirementObserver>,
        tracker: RetirementSettlement,
    ) -> Self {
        Self {
            key,
            observer,
            _tracker: tracker,
            is_pending: true,
        }
    }

    fn settle(mut self, result: &Result<(), Error>) {
        // Disarm before publishing so an observer panic cannot publish a
        // second, misleading cancellation while unwinding this guard.
        self.is_pending = false;
        if let Err(error) = result {
            self.observer
                .observe(&self.key, RetirementFailureStage::TerminalCleanup, error);
        }
    }
}

impl Drop for RetirementTerminalSettlement {
    fn drop(&mut self) {
        if self.is_pending {
            self.observer.observe(
                &self.key,
                RetirementFailureStage::TerminalCleanup,
                &Error::cancelled(),
            );
        }
    }
}

impl PendingRetirement {
    pub(super) fn new(
        managed: Arc<dyn ManagedHandle>,
        settlement: RetirementSettlement,
        event_bus: Arc<EventBus<ResourceEvent>>,
        metrics: Option<ResourceOpsMetrics>,
        origin: RetirementOrigin,
    ) -> Self {
        let key = managed.resource_key();
        let observer = Arc::new(RetirementObserver {
            event_bus,
            metrics,
            origin,
        });
        Self {
            managed,
            terminal: Some(RetirementTerminalSettlement::new(
                key,
                Arc::clone(&observer),
                settlement,
            )),
            observer,
        }
    }

    #[tracing::instrument(skip_all, fields(resource.key = %self.managed.resource_key()))]
    async fn close(mut self, release_queue: Arc<ReleaseQueue>) -> Option<RetirementFailure> {
        let key = self.managed.resource_key();
        let origin = self.observer.origin;
        let maintenance = self.managed.join_maintenance().await;
        if let Err(error) = &maintenance {
            self.observer
                .observe(&key, RetirementFailureStage::Maintenance, error);
        }
        let terminal = if let Some(settlement) = self.terminal.take() {
            let managed = Arc::clone(&self.managed);
            let submission = release_queue.submit_coordinator(move || {
                Box::pin(async move {
                    let result = managed.close_retained().await;
                    settlement.settle(&result);
                    result
                })
            });
            match submission {
                Ok(submission) => submission.wait().await,
                Err(error) => Err(error),
            }
        } else {
            let error = Error::cancelled();
            tracing::error!(resource.key = %key, "retirement terminal settlement was already consumed");
            Err(error)
        };
        match terminal {
            Ok(SubmissionOutcome::Completed) => maintenance.err().map(|source| RetirementFailure {
                key,
                origin,
                source,
            }),
            Ok(SubmissionOutcome::Deferred) => {
                let source = maintenance.err().unwrap_or_else(|| {
                    Error::permanent(
                        "root retirement supervisor unexpectedly deferred terminal cleanup",
                    )
                });
                Some(RetirementFailure {
                    key,
                    origin,
                    source,
                })
            },
            Err(terminal_error) => Some(RetirementFailure {
                key,
                origin,
                source: maintenance.err().unwrap_or(terminal_error),
            }),
        }
    }

    pub(super) fn abandon(self, reason: &'static str) {
        let key = self.managed.resource_key();
        tracing::warn!(
            resource.key = %key,
            reason,
            "resource retirement abandoned before supervisor admission"
        );
        drop(self);
    }
}

impl Drop for PendingRetirement {
    fn drop(&mut self) {
        self.managed.abort_maintenance();
    }
}

#[cfg(test)]
mod tests {
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

    fn assert_one_cancelled_terminal_event(
        events: &mut nebula_eventbus::Subscriber<ResourceEvent>,
    ) {
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
}
