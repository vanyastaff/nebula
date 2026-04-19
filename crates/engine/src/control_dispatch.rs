//! Engine-owned [`ControlDispatch`] implementation — ADR-0008 follow-ups A2
//! (Start / Resume / Restart) and A3 (Cancel / Terminate).
//!
//! The [`ControlConsumer`] (skeleton landed in A1) drains
//! `execution_control_queue` rows and hands each typed command to an
//! implementation of [`ControlDispatch`]. [`EngineControlDispatch`] wires the
//! `Start` / `Resume` / `Restart` paths into the engine so that a POST to
//! `/executions` actually causes node execution — closing the §4.5 gap named
//! in #332. The `Cancel` / `Terminate` path closes the §12.2 / §13-step-5
//! symmetric gap named in #330: the durable `Cancel` signal the API's
//! `cancel_execution` handler enqueues now reaches the live frontier loop
//! via [`WorkflowEngine::cancel_execution`].
//!
//! ## Idempotency contract (ADR-0008 §5)
//!
//! Control-queue delivery is at-least-once: the ack path on `mark_completed`
//! may fail after a successful dispatch, and the reclaim path (B1) will
//! redeliver. Each dispatch method guards against re-delivery through one of
//! two mechanisms:
//!
//! - **Start / Resume / Restart** short-circuit on persisted status. A command arriving for an
//!   already-terminal execution is `Ok(())`; a command arriving for a `Running` / `Cancelling`
//!   execution is `Ok(())` (a sibling runner already owns the dispatch). A race where a second
//!   dispatcher wins the lease between our read and the engine's own lease acquire surfaces as
//!   [`EngineError::Leased`], which this impl maps to `Ok(())` so the same execution is not fenced
//!   as a consumer failure.
//!
//! - **Cancel / Terminate** always signal the engine's cancel registry (except for orphan commands,
//!   which are [`ControlDispatchError::Rejected`]). The underlying
//!   [`tokio_util::sync::CancellationToken::cancel`] is idempotent per token, and a missing
//!   registry entry — cross-runner case or this runner already cleaned up — is a no-op that returns
//!   [`WorkflowEngine::cancel_execution`] `= false` without side effects. Short-circuiting on
//!   terminal status would leave a live frontier loop orphaned after the API handler's CAS
//!   transitioned the row to `Cancelled` in the same logical operation as the enqueue (canon §12.2
//!   / §13 step 5) — the durable state would say the run is over while the in-process `JoinSet`
//!   kept waiting on a slow handler.
//!
//! The authoritative single-runner fence still lives inside
//! [`WorkflowEngine::resume_execution`] (ADR-0015 lease lifecycle); this
//! module just forwards commands and collapses the resulting errors into the
//! [`ControlDispatch`] contract.
//!
//! [`ControlConsumer`]: crate::ControlConsumer
//! [`ControlDispatch`]: crate::ControlDispatch
//! [`WorkflowEngine::resume_execution`]: crate::WorkflowEngine::resume_execution
//! [`WorkflowEngine::cancel_execution`]: crate::WorkflowEngine::cancel_execution

use std::sync::Arc;

use async_trait::async_trait;
use nebula_core::id::ExecutionId;
use nebula_execution::ExecutionStatus;
use nebula_storage::ExecutionRepo;

use crate::{
    WorkflowEngine,
    control_consumer::{ControlDispatch, ControlDispatchError},
    error::EngineError,
};

/// Engine-owned [`ControlDispatch`] implementation.
///
/// Holds a shared [`WorkflowEngine`] plus a handle to the execution repo so
/// the dispatch methods can read the current status for the idempotency
/// check without entering the engine's lease scope on a re-delivered
/// command. Construction mirrors how a composition root wires the API and
/// engine together — they share the same `ExecutionRepo` so status reads
/// from either side agree.
///
/// See the module docs for the idempotency contract and ADR-0008 §5 for the
/// canon rules this impl honors.
#[derive(Clone)]
pub struct EngineControlDispatch {
    engine: Arc<WorkflowEngine>,
    execution_repo: Arc<dyn ExecutionRepo>,
}

impl EngineControlDispatch {
    /// Build a new dispatch bound to the given engine and execution repo.
    ///
    /// The caller MUST pass the same `execution_repo` the engine was
    /// configured with via [`WorkflowEngine::with_execution_repo`]; otherwise
    /// the idempotency read and the engine's internal CAS see divergent
    /// state.
    ///
    /// [`WorkflowEngine::with_execution_repo`]: crate::WorkflowEngine::with_execution_repo
    #[must_use]
    pub fn new(engine: Arc<WorkflowEngine>, execution_repo: Arc<dyn ExecutionRepo>) -> Self {
        Self {
            engine,
            execution_repo,
        }
    }

    /// Read the persisted [`ExecutionStatus`] for an execution, returning
    /// `None` if the row does not exist.
    async fn read_status(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Option<ExecutionStatus>, ControlDispatchError> {
        let state = self
            .execution_repo
            .get_state(execution_id)
            .await
            .map_err(|e| {
                ControlDispatchError::Internal(format!(
                    "read execution state for idempotency guard: {e}"
                ))
            })?;
        match state {
            None => Ok(None),
            Some((_version, json)) => match json.get("status") {
                Some(s) => serde_json::from_value::<ExecutionStatus>(s.clone())
                    .map(Some)
                    .map_err(|e| {
                        ControlDispatchError::Internal(format!(
                            "execution {execution_id}: status field did not deserialize: {e}"
                        ))
                    }),
                None => Err(ControlDispatchError::Internal(format!(
                    "execution {execution_id}: persisted state has no `status` field"
                ))),
            },
        }
    }

    /// Drive an execution that is `Created` or `Paused` through the engine's
    /// resume path. Shared by `dispatch_start`, `dispatch_resume`, and
    /// `dispatch_restart` — the three commands converge on the same engine
    /// entry today because the engine does not yet distinguish a
    /// `restart-from-input` rewind from a normal resume (true rewind
    /// requires durable output purge — tracked separately).
    async fn drive(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        match self.engine.resume_execution(execution_id).await {
            Ok(_) => Ok(()),
            // Concurrent dispatcher already holds the lease — the canonical
            // ADR-0008 §5 idempotency outcome. Returning `Ok(())` here prevents
            // the consumer from marking the row `Failed`; the lease holder
            // owns the terminal transition.
            Err(EngineError::Leased { .. }) => Ok(()),
            Err(e) => {
                // Last-ditch idempotency guard: re-read the row in case a
                // sibling dispatcher drove it to a terminal state between our
                // initial read and the engine's own `get_state` inside
                // `resume_execution`. This catches both the "already terminal"
                // `PlanningFailed` that `resume_execution` surfaces on re-entry
                // and the race where a parallel `Cancel` beat us to the row.
                if let Ok(Some(status)) = self.read_status(execution_id).await
                    && (status.is_terminal() || matches!(status, ExecutionStatus::Cancelling))
                {
                    return Ok(());
                }
                Err(ControlDispatchError::Internal(format!(
                    "engine dispatch failed for {execution_id}: {e}"
                )))
            },
        }
    }
}

#[async_trait]
impl ControlDispatch for EngineControlDispatch {
    async fn dispatch_start(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        match self.read_status(execution_id).await? {
            None => Err(ControlDispatchError::Rejected(format!(
                "execution {execution_id} not found — start command orphaned"
            ))),
            // Already past the Created gate: either the engine is driving it
            // (Running / Cancelling) or it has already reached a terminal
            // outcome. Re-delivered Start is a no-op per ADR-0008 §5.
            Some(
                ExecutionStatus::Running
                | ExecutionStatus::Cancelling
                | ExecutionStatus::Completed
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
                | ExecutionStatus::TimedOut,
            ) => Ok(()),
            Some(ExecutionStatus::Created | ExecutionStatus::Paused) => {
                self.drive(execution_id).await
            },
        }
    }

    async fn dispatch_resume(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        // `Resume` and `Start` converge on the same engine entry today — see
        // the `drive` docs. The idempotency read mirrors `dispatch_start`.
        match self.read_status(execution_id).await? {
            None => Err(ControlDispatchError::Rejected(format!(
                "execution {execution_id} not found — resume command orphaned"
            ))),
            Some(
                ExecutionStatus::Running
                | ExecutionStatus::Cancelling
                | ExecutionStatus::Completed
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
                | ExecutionStatus::TimedOut,
            ) => Ok(()),
            Some(ExecutionStatus::Created | ExecutionStatus::Paused) => {
                self.drive(execution_id).await
            },
        }
    }

    async fn dispatch_restart(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        // Per ADR-0008 §5 restart docs: "double-restart rewinds twice". A true
        // rewind-from-input restart requires durable output purge plus a
        // restart counter — neither exists yet. For A2, treat restart as a
        // re-entrant drive of the engine's resume path and honor the same
        // terminal / running idempotency outcomes.
        //
        // Restart-of-terminal intentionally errors so operators see the gap
        // in the `execution_control_queue.error_message` rather than the
        // command silently succeeding and not actually restarting anything.
        match self.read_status(execution_id).await? {
            None => Err(ControlDispatchError::Rejected(format!(
                "execution {execution_id} not found — restart command orphaned"
            ))),
            Some(ExecutionStatus::Running | ExecutionStatus::Cancelling) => Ok(()),
            Some(
                status @ (ExecutionStatus::Completed
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
                | ExecutionStatus::TimedOut),
            ) => Err(ControlDispatchError::Rejected(format!(
                "execution {execution_id} is already {status}; rewind-from-input restart \
                 requires durable output purge — not yet implemented, tracked under ADR-0008 \
                 follow-up"
            ))),
            Some(ExecutionStatus::Created | ExecutionStatus::Paused) => {
                self.drive(execution_id).await
            },
        }
    }

    async fn dispatch_cancel(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        // ADR-0008 A3 — every non-orphan `Cancel` signals the engine's
        // cancel registry, regardless of the persisted status.
        //
        // The API handler's `cancel_execution` writes the row to `Cancelled`
        // in the same logical operation as the enqueue (canon §12.2 / §13
        // step 5), so by the time the consumer drains this command, the
        // read here will typically report a terminal status even for a
        // live frontier loop. Short-circuiting on terminal would leave a
        // running slow handler orphaned — the durable state says the run
        // is over, but the in-process JoinSet is still blocked in a node.
        //
        // `engine.cancel_execution` is idempotent in both dimensions we
        // care about: the underlying `CancellationToken::cancel()` is a
        // no-op on a token that is already cancelled, and a missing
        // registry entry (cross-runner, or this runner already cleaned up)
        // returns `false` without side effects. Signalling always is the
        // honest minimum: it closes the live-loop gap and is safe under
        // at-least-once redelivery (ADR-0008 §5).
        match self.read_status(execution_id).await? {
            // Producer bug: queue row written without the execution row (or a
            // row that disappeared between enqueue and drain). Surface so the
            // diagnosis lands on `execution_control_queue.error_message`.
            None => Err(ControlDispatchError::Rejected(format!(
                "execution {execution_id} not found — cancel command orphaned"
            ))),
            Some(status) => {
                let signalled = self.engine.cancel_execution(execution_id);
                tracing::debug!(
                    %execution_id,
                    %status,
                    signalled,
                    "control-queue: Cancel dispatched (ADR-0008 A3) — signalled local runner={signalled}"
                );
                Ok(())
            },
        }
    }

    async fn dispatch_terminate(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        // ADR-0008 names `Terminate` "forced termination", but there is no
        // distinct forced-shutdown path in the engine today — the frontier
        // loop aborts in-flight `JoinSet` tasks via the same cooperative
        // `CancellationToken` that `Cancel` trips. Treating `Terminate` as a
        // synonym for `Cancel` is the honest minimum: the operator-visible
        // contract is identical (in-flight work aborts, state reaches a
        // terminal `Cancelled`), and the capability gap — process-level kill
        // or task-set abort — is tracked separately as a future chip. Do not
        // emit half-implemented forced-abort machinery here (canon §4.5).
        //
        // See ADR-0016 (cancel-registry and cooperative-cancel contract) for
        // the design rationale and the upgrade path to a true forced-shutdown
        // distinction.
        self.dispatch_cancel(execution_id).await
    }
}
