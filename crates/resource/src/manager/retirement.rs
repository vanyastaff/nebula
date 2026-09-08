//! Bounded, manager-owned orchestration for resource-row retirement.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::{StreamExt as _, stream::FuturesUnordered};
use nebula_core::ResourceKey;
use tokio::sync::mpsc;

use crate::{error::Error, registry::ManagedHandle, release_queue::ReleaseQueue};

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

fn preserve_first_failure(
    first_failure: &mut Option<RetirementFailure>,
    failure: Option<RetirementFailure>,
) {
    if first_failure.is_none() {
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
        if let Some(failure) = active.next().await {
            preserve_first_failure(first_failure, failure);
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
                    while let Some(failure) = active.next().await {
                        preserve_first_failure(&mut first_failure, failure);
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
                    while let Some(failure) = active.next().await {
                        preserve_first_failure(&mut first_failure, failure);
                    }
                    return first_failure;
                },
                None => return first_failure,
            },
            Some(failure) = active.next(), if !active.is_empty() => {
                preserve_first_failure(&mut first_failure, failure);
            },
        }
    }
}

#[must_use]
pub(super) struct RetirementFailure {
    pub(super) key: ResourceKey,
    pub(super) source: Error,
}

/// Fenced row ownership held before bounded supervisor admission.
pub(super) struct PendingRetirement {
    managed: Arc<dyn ManagedHandle>,
    settlement: Option<RetirementSettlement>,
}

impl PendingRetirement {
    pub(super) fn new(managed: Arc<dyn ManagedHandle>, settlement: RetirementSettlement) -> Self {
        Self {
            managed,
            settlement: Some(settlement),
        }
    }

    #[tracing::instrument(skip_all, fields(resource.key = %self.managed.resource_key()))]
    async fn close(mut self, release_queue: Arc<ReleaseQueue>) -> Option<RetirementFailure> {
        let key = self.managed.resource_key();
        let maintenance = self.managed.join_maintenance().await;
        let terminal = if let Some(settlement) = self.settlement.take() {
            let managed = Arc::clone(&self.managed);
            let receipt = release_queue.submit_coordinator(move || {
                Box::pin(async move {
                    let _settlement = settlement;
                    managed.close_retained().await
                })
            });
            receipt.await.unwrap_or_else(|_| Err(Error::cancelled()))
        } else {
            Err(Error::cancelled())
        };
        maintenance
            .and(terminal)
            .err()
            .map(|source| RetirementFailure { key, source })
    }

    pub(super) fn abandon(self, reason: &'static str) {
        tracing::warn!(
            resource.key = %self.managed.resource_key(),
            reason,
            "resource retirement abandoned before supervisor admission"
        );
    }
}

impl Drop for PendingRetirement {
    fn drop(&mut self) {
        self.managed.abort_maintenance();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_handle_observes_completion_without_a_parallel_receipt() {
        let handle = tokio::spawn(async { None });
        let owner = RetirementSupervisorHandle {
            handle: Some(handle),
        };
        let result = owner.join_bounded(std::time::Duration::from_secs(1)).await;
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
        let result = owner.join_bounded(std::time::Duration::from_secs(1)).await;
        assert!(matches!(result, Err(RetirementSupervisorJoinError::Failed)));
    }
}
