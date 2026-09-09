//! One shutdown snapshot, publisher, and absolute budget across caller cancellation.

use std::sync::Arc;

use tokio::{sync::Notify, task::JoinHandle, time::Instant};

use super::{
    ShutdownConfig,
    retirement::{PendingRetirement, PublishBatchError, RetirementSupervisor},
    shutdown::{ShutdownError, ShutdownReport, wait_for_tracker_drain},
};

pub(super) enum ShutdownState {
    Open,
    Draining(ShutdownSession),
    Finishing(JoinHandle<Result<ShutdownReport, ShutdownError>>),
    Finished,
}

impl ShutdownState {
    pub(super) fn abort(&mut self) {
        if let Self::Finishing(task) = self {
            task.abort();
        }
    }
}

pub(super) struct ShutdownSession {
    pub(super) config: ShutdownConfig,
    started: Instant,
    publication: Publication,
}

enum Publication {
    Pending(Publisher),
    Published,
    Failed(PublicationFailure),
}

enum PublicationFailure {
    Batch(PublishBatchError),
    Task,
}

struct Publisher(JoinHandle<Result<(), PublishBatchError>>);

impl Drop for Publisher {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) enum DrainFailure {
    TimedOut { outstanding: u64 },
    Publication,
}

impl ShutdownSession {
    /// Called under admission before the first await; task ownership never belongs to the caller.
    pub(super) fn start(
        config: ShutdownConfig,
        started: Instant,
        retirements: Vec<PendingRetirement>,
        supervisor: Arc<RetirementSupervisor>,
    ) -> Self {
        tracing::debug!(
            rows = retirements.len(),
            "resource manager: starting shutdown snapshot publisher"
        );
        // Saturating duration arithmetic also admits Duration::MAX without an Instant overflow.
        let envelope = config
            .drain_timeout
            .saturating_add(config.release_queue_timeout);
        let publisher = tokio::spawn(async move {
            if retirements.is_empty() {
                return Ok(());
            }
            supervisor
                .publish_batch_bounded(retirements, envelope.saturating_sub(started.elapsed()))
                .await
        });
        Self {
            config,
            started,
            publication: Publication::Pending(Publisher(publisher)),
        }
    }

    pub(super) fn terminal_budget(&self) -> std::time::Duration {
        self.config
            .drain_timeout
            .saturating_add(self.config.release_queue_timeout)
            .saturating_sub(self.started.elapsed())
            .min(self.config.release_queue_timeout)
    }

    pub(super) async fn drain(
        &mut self,
        tracker: &Arc<(std::sync::atomic::AtomicU64, Notify)>,
    ) -> Result<(), DrainFailure> {
        loop {
            let remaining = self
                .config
                .drain_timeout
                .saturating_sub(self.started.elapsed());
            match &mut self.publication {
                Publication::Failed(_) => return Err(DrainFailure::Publication),
                Publication::Published => {
                    return wait_for_tracker_drain(tracker, remaining)
                        .await
                        .map_err(|outstanding| DrainFailure::TimedOut { outstanding });
                },
                Publication::Pending(publisher) => {
                    // JoinHandle and tracker drain are cancellation safe. The stored publisher
                    // survives a dropped driver, and a completed failure wins over a drain timeout.
                    tokio::select! {
                        biased;
                        result = &mut publisher.0 => self.publication = publication_result(result),
                        result = wait_for_tracker_drain(tracker, remaining) => {
                            return result.map_err(|outstanding| DrainFailure::TimedOut { outstanding });
                        },
                    }
                },
            }
        }
    }

    pub(super) async fn settle_publication(
        mut self,
        budget: std::time::Duration,
    ) -> (ShutdownConfig, Option<ShutdownError>) {
        if let Publication::Pending(publisher) = &mut self.publication {
            if let Ok(result) = tokio::time::timeout(budget, &mut publisher.0).await {
                self.publication = publication_result(result);
            } else {
                publisher.0.abort();
                // A send may have committed before the abort. Observe the join result:
                // published owners stay in the supervisor; returned owners are abandoned once.
                let result = (&mut publisher.0).await;
                self.publication = publication_result(result);
                publication_error(self.publication, self.config.release_queue_timeout);
                let error = ShutdownError::ReleaseQueueTimeout {
                    timeout: self.config.release_queue_timeout,
                };
                return (self.config, Some(error));
            }
        }
        let error = publication_error(self.publication, self.config.release_queue_timeout);
        (self.config, error)
    }
}

fn publication_error(
    publication: Publication,
    timeout: std::time::Duration,
) -> Option<ShutdownError> {
    match publication {
        Publication::Failed(PublicationFailure::Batch(PublishBatchError::TimedOut(owners))) => {
            for owner in owners {
                owner.abandon("shutdown publication timed out");
            }
            Some(ShutdownError::ReleaseQueueTimeout { timeout })
        },
        Publication::Failed(PublicationFailure::Batch(PublishBatchError::Closed(owners))) => {
            for owner in owners {
                owner.abandon("retirement supervisor closed before publication");
            }
            Some(ShutdownError::RetirementSupervisorFailed)
        },
        Publication::Failed(PublicationFailure::Task) => {
            Some(ShutdownError::RetirementSupervisorFailed)
        },
        Publication::Published => None,
        Publication::Pending(_) => Some(ShutdownError::RetirementSupervisorFailed),
    }
}

fn publication_result(
    result: Result<Result<(), PublishBatchError>, tokio::task::JoinError>,
) -> Publication {
    match result {
        Ok(Ok(())) => {
            tracing::debug!("resource manager: shutdown snapshot publication accepted");
            Publication::Published
        },
        Ok(Err(error)) => Publication::Failed(PublicationFailure::Batch(error)),
        Err(error) => {
            tracing::error!(
                cancelled = error.is_cancelled(),
                panicked = error.is_panic(),
                "manager-owned shutdown publisher failed"
            );
            Publication::Failed(PublicationFailure::Task)
        },
    }
}
