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
#[path = "retirement_tests.rs"]
mod tests;
