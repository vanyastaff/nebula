//! Queue-owned credential-hook admission, receipt, and settlement protocol.
//!
//! Admission transfers terminal accounting to the cleanup queue before the
//! caller may stop observing its receipt. Retained cleanup settles separately
//! so framework cleanup faults cannot overwrite the provider hook outcome.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    error::Error,
    release_queue::ReleaseSubmission,
    resource::{Provider, TeardownReason},
    runtime::managed::ManagedResource,
    topology::Topology,
};

use super::{RetiredCleanupObserver, RetiredEntriesGuard};

/// Queue-owned terminal observation for one admitted credential hook.
#[derive(Debug)]
pub(crate) enum SlotHookObservation {
    Completed,
    Failed(crate::ErrorKind),
    TimedOut(crate::ErrorKind),
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RetiredCleanupObservation {
    Failed(crate::ErrorKind),
    TimedOut,
    Abandoned,
}

type SlotHookObserver = Box<dyn FnOnce(SlotHookObservation) + Send>;

pub(super) struct RetiredCleanupSettlement {
    observer: Option<RetiredCleanupObserver>,
}

impl RetiredCleanupSettlement {
    pub(super) fn new(observer: Option<RetiredCleanupObserver>) -> Self {
        Self { observer }
    }

    pub(super) fn settle(mut self, observation: Option<RetiredCleanupObservation>) {
        if let (Some(observer), Some(observation)) = (self.observer.take(), observation) {
            observer(observation);
        }
    }
}

impl Drop for RetiredCleanupSettlement {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.take() {
            observer(RetiredCleanupObservation::Abandoned);
        }
    }
}

struct SlotHookSettlementState {
    is_admitted: bool,
    pending: Option<SlotHookObservation>,
    observer: Option<SlotHookObserver>,
}

/// Queue-owned exactly-once settlement token. Dropping it before an explicit
/// terminal result publishes `Abandoned`, including queued-task loss and
/// cancellation while the provider future is pending.
pub(crate) struct SlotHookSettlement {
    shared: Arc<Mutex<SlotHookSettlementState>>,
    cleanup_observer: Option<RetiredCleanupObserver>,
    is_settled: bool,
}

pub(crate) struct SlotHookAdmission {
    shared: Arc<Mutex<SlotHookSettlementState>>,
}

pub(crate) struct AcceptedSlotHook {
    pub(super) receipt: SlotHookReceipt,
    pub(super) has_started: Arc<AtomicBool>,
}

pub(super) enum SlotHookReceipt {
    Await(tokio::sync::oneshot::Receiver<SlotHookWaitOutcome>),
    Deferred,
}

pub(crate) enum SlotHookWaitOutcome {
    Completed,
    Deferred(SlotHookDeferral),
    Abandoned,
    Failed(Error),
    TimedOut(Error),
}

enum SlotHookTerminalOutcome {
    Completed,
    Failed(Error),
    TimedOut(Error),
}

impl SlotHookTerminalOutcome {
    fn into_wait_and_observation(self) -> (SlotHookWaitOutcome, SlotHookObservation) {
        match self {
            Self::Completed => (
                SlotHookWaitOutcome::Completed,
                SlotHookObservation::Completed,
            ),
            Self::Failed(error) => {
                let observation = SlotHookObservation::Failed(error.kind().clone());
                (SlotHookWaitOutcome::Failed(error), observation)
            },
            Self::TimedOut(error) => {
                let observation = SlotHookObservation::TimedOut(error.kind().clone());
                (SlotHookWaitOutcome::TimedOut(error), observation)
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotHookDeferral {
    CleanupContext,
    /// The accepted hook had not started when the post-admission observation
    /// budget elapsed.
    ObservationTimedOut,
}

impl SlotHookSettlement {
    pub(crate) fn new(observer: SlotHookObserver) -> (Self, SlotHookAdmission) {
        let shared = Arc::new(Mutex::new(SlotHookSettlementState {
            is_admitted: false,
            pending: None,
            observer: Some(observer),
        }));
        (
            Self {
                shared: Arc::clone(&shared),
                cleanup_observer: None,
                is_settled: false,
            },
            SlotHookAdmission { shared },
        )
    }

    pub(crate) fn with_cleanup_observer(mut self, observer: RetiredCleanupObserver) -> Self {
        self.cleanup_observer = Some(observer);
        self
    }

    pub(crate) fn settle(mut self, hook: SlotHookObservation) -> Option<RetiredCleanupObserver> {
        self.is_settled = true;
        self.publish(hook);
        self.cleanup_observer.take()
    }

    fn publish(&self, observation: SlotHookObservation) {
        let callback = {
            let mut state = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.is_admitted {
                state
                    .observer
                    .take()
                    .map(|observer| (observer, observation))
            } else {
                state.pending = Some(observation);
                None
            }
        };
        if let Some((observer, observation)) = callback {
            observer(observation);
        }
    }
}

impl Drop for SlotHookSettlement {
    fn drop(&mut self) {
        if !self.is_settled {
            self.publish(SlotHookObservation::Abandoned);
        }
    }
}

impl SlotHookAdmission {
    pub(crate) fn admit(self) {
        let callback = {
            let mut state = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.is_admitted = true;
            match (state.observer.take(), state.pending.take()) {
                (Some(observer), Some(observation)) => Some((observer, observation)),
                (observer, observation) => {
                    state.observer = observer;
                    state.pending = observation;
                    None
                },
            }
        };
        if let Some((observer, observation)) = callback {
            observer(observation);
        }
    }
}

impl AcceptedSlotHook {
    fn classify_receipt(
        result: Result<SlotHookWaitOutcome, tokio::sync::oneshot::error::RecvError>,
    ) -> SlotHookWaitOutcome {
        result.unwrap_or(SlotHookWaitOutcome::Abandoned)
    }

    /// Waits until the caller's absolute deadline while the hook is queued.
    /// Once queue execution starts, the hook's own bounded execution budget
    /// determines its terminal result. Receipt completion is biased over the
    /// deadline so a result ready on the same scheduler turn wins.
    pub(crate) async fn wait_until(self, deadline: tokio::time::Instant) -> SlotHookWaitOutcome {
        match self.receipt {
            SlotHookReceipt::Deferred => {
                SlotHookWaitOutcome::Deferred(SlotHookDeferral::CleanupContext)
            },
            SlotHookReceipt::Await(mut receipt) => {
                tokio::select! {
                    biased;
                    result = &mut receipt => Self::classify_receipt(result),
                    () = tokio::time::sleep_until(deadline) => {
                        // The queue task publishes `has_started` with Release
                        // before entering the bounded author hook. Acquire here
                        // ensures that once execution owns the hook, this
                        // observer waits for its terminal bounded result rather
                        // than racing an equal-duration observation deadline.
                        if self.has_started.load(Ordering::Acquire) {
                            Self::classify_receipt(receipt.await)
                        } else {
                            SlotHookWaitOutcome::Deferred(SlotHookDeferral::ObservationTimedOut)
                        }
                    },
                }
            },
        }
    }
}

impl<R> ManagedResource<R>
where
    R: Provider,
    R::Topology: Topology<R>,
{
    /// Borrows the live topology and invokes the per-entry credential hook —
    /// [`Provider::on_credential_refresh`] when `refresh` is `true`,
    /// [`Provider::on_credential_revoke`] otherwise — against this resource's
    /// instances.
    ///
    /// The dispatch is topology-specific (resident reconcile vs pool idle
    /// fan-out) and lives behind [`Topology::dispatch_credential_hook`]; the
    /// resource handle the hook needs is supplied from `self.resource`.
    ///
    /// # Cancel Safety
    ///
    /// Admission is synchronous. Before successful submission the caller still
    /// owns the settlement token and a rejection is retryable. After submission
    /// the queued task owns both the hook and its exactly-once terminal
    /// settlement; dropping an [`AcceptedSlotHook`] only stops observing its
    /// receipt and cannot cancel the hook or its terminal accounting.
    pub(crate) fn submit_slot_hook(
        self: &Arc<Self>,
        slot: &str,
        refresh: bool,
        hook_timeout: std::time::Duration,
        settlement: SlotHookSettlement,
        admission: SlotHookAdmission,
    ) -> Result<AcceptedSlotHook, Error> {
        let managed = Arc::clone(self);
        let slot = slot.to_owned();
        let has_started = Arc::new(AtomicBool::new(false));
        let task_has_started = Arc::clone(&has_started);
        let (hook_result_tx, hook_result_rx) = tokio::sync::oneshot::channel();
        let submission = self.release_queue.submit_coordinator(move || {
            Box::pin(async move {
                task_has_started.store(true, Ordering::Release);
                let _retirement = RetiredEntriesGuard(Arc::clone(&managed));
                let guarded_hook = crate::hook_guard::guard_author_hook(
                    hook_timeout,
                    managed.topology.dispatch_credential_hook(
                        &managed.resource,
                        &managed.store,
                        &managed.retained,
                        &slot,
                        refresh,
                    ),
                )
                .await;
                let hook_terminal = match guarded_hook {
                    Ok(Ok(())) => SlotHookTerminalOutcome::Completed,
                    Ok(Err(crate::topology::HookFault::Failed(error))) => {
                        SlotHookTerminalOutcome::Failed(error)
                    },
                    Ok(Err(crate::topology::HookFault::TimedOut)) => {
                        SlotHookTerminalOutcome::TimedOut(Error::backpressure(
                            "credential topology hook timed out",
                        ))
                    },
                    Err(fault) => {
                        fault.observe(&R::key(), "rotation");
                        match fault {
                            crate::hook_guard::HookFault::Panicked => {
                                SlotHookTerminalOutcome::Failed(Error::permanent(
                                    "credential topology hook panicked",
                                ))
                            },
                            crate::hook_guard::HookFault::TimedOut => {
                                SlotHookTerminalOutcome::TimedOut(Error::backpressure(
                                    "credential topology hook timed out",
                                ))
                            },
                        }
                    },
                };
                let (hook_outcome, hook_observation) = hook_terminal.into_wait_and_observation();
                let cleanup_settlement =
                    RetiredCleanupSettlement::new(settlement.settle(hook_observation));
                let _ = hook_result_tx.send(hook_outcome);

                let cleanup =
                    match tokio::time::timeout(crate::hook_guard::MAX_TEARDOWN_CEILING, async {
                        loop {
                            let (entries, blocked) = managed.retained.drain_retired().into_parts();
                            let mut batch = super::super::destroy_batch::DestroyBatch::new(
                                Arc::clone(&managed),
                                Vec::new(),
                                TeardownReason::Evicted,
                            );
                            batch.extend_retained(entries);
                            batch.run().await?;
                            let Some(blocked) = blocked else {
                                return Ok::<(), Error>(());
                            };
                            tracing::debug!(
                                resource.key = %R::key(),
                                reason = ?blocked.reason(),
                                "credential rotation waits for retired generations only"
                            );
                            managed
                                .retained
                                .wait_retired_ready()
                                .await
                                .map_err(|blocked| {
                                    tracing::error!(
                                        resource.key = %R::key(),
                                        reason = ?blocked.reason(),
                                        "retained lease accounting prevents rotation cleanup"
                                    );
                                    Error::permanent("retained lease accounting is unavailable")
                                })?;
                        }
                    })
                    .await
                    {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => {
                            tracing::warn!(
                                error.kind = ?error.kind(),
                                resource.key = %R::key(),
                                cleanup.outcome = "failed",
                                "retired generation cleanup failed after credential hook settlement"
                            );
                            Some(RetiredCleanupObservation::Failed(error.kind().clone()))
                        },
                        Err(_) => {
                            tracing::warn!(
                                resource.key = %R::key(),
                                cleanup.outcome = "timed_out",
                                "retired generation cleanup exceeded its rotation budget"
                            );
                            Some(RetiredCleanupObservation::TimedOut)
                        },
                    };
                cleanup_settlement.settle(cleanup);
                Ok(())
            })
        })?;
        let receipt = match submission {
            ReleaseSubmission::Await(queue_receipt) => {
                drop(queue_receipt);
                SlotHookReceipt::Await(hook_result_rx)
            },
            ReleaseSubmission::Deferred => SlotHookReceipt::Deferred,
        };
        admission.admit();
        Ok(AcceptedSlotHook {
            receipt,
            has_started,
        })
    }
}
