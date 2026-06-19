//! Engine-owned [`ControlDispatch`] implementation — follow-ups A2
//! (Start / Resume / Restart) and A3 (Cancel / Terminate).
//!
//! The [`ControlConsumer`] (skeleton landed in A1) drains
//! `execution_control_queue` rows and hands each typed command to an
//! implementation of [`ControlDispatch`]. [`EngineControlDispatch`] wires the
//! `Start` / `Resume` / `Restart` paths into the engine so that a POST to
//! `/executions` actually causes node execution — closing the gap named
//! in #332. The `Cancel` / `Terminate` path closes the symmetric cancel gap named in #330: the durable `Cancel` signal the API's
//! `cancel_execution` handler enqueues now reaches the live frontier loop
//! via [`WorkflowEngine::cancel_execution`].
//!
//! ## Idempotency contract
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
//!   transitioned the row to `Cancelled` in the same logical operation as the enqueue (control-queue
//!   cancel path) — the durable state would say the run is over while the in-process `JoinSet`
//!   kept waiting on a slow handler.
//!
//! The authoritative single-runner fence still lives inside
//! [`WorkflowEngine::resume_execution`] (lease lifecycle); this
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
use nebula_storage_port::{Scope, store::ExecutionStore};

use crate::{
    WorkflowEngine,
    control_consumer::{ControlDispatch, ControlDispatchError},
    engine::SatisfyOutcome,
    error::EngineError,
    event::ExecutionEvent,
};

/// Engine-owned [`ControlDispatch`] implementation.
///
/// Holds a shared [`WorkflowEngine`] plus a scoped [`ExecutionStore`]
/// handle so the dispatch methods can read the current status for the
/// idempotency check without entering the engine's lease scope on a
/// re-delivered command. Construction mirrors how a composition root
/// wires the API and engine together — they share the same execution
/// store so status reads from either side agree.
///
/// See the module docs for the idempotency contract and for the
/// canon rules this impl honors.
#[derive(Clone)]
pub struct EngineControlDispatch {
    engine: Arc<WorkflowEngine>,
    execution: Arc<dyn ExecutionStore>,
}

impl EngineControlDispatch {
    /// Build a new dispatch reading status through the spec-16
    /// [`ExecutionStore`] port.
    ///
    /// The caller MUST pass the same scoped store the engine was
    /// configured with via [`WorkflowEngine::with_execution_stores`] so
    /// the idempotency read and the engine's internal CAS observe the
    /// same row.
    ///
    /// [`WorkflowEngine::with_execution_stores`]: crate::WorkflowEngine::with_execution_stores
    #[must_use]
    pub fn new(engine: Arc<WorkflowEngine>, execution: Arc<dyn ExecutionStore>) -> Self {
        Self { engine, execution }
    }

    /// Read the persisted [`ExecutionStatus`] for an execution under the given
    /// tenant `scope`, returning `None` if the row does not exist.
    ///
    /// `scope` MUST be the per-message scope from `ControlMsg.scope` so that
    /// execution rows belonging to a different tenant are never visible here
    /// (cross-tenant isolation invariant #7).
    async fn read_status(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<Option<ExecutionStatus>, ControlDispatchError> {
        let json = self
            .execution
            .get(scope, &execution_id.to_string())
            .await
            .map_err(|e| {
                ControlDispatchError::Internal(format!(
                    "read execution state for idempotency guard: {e}"
                ))
            })?
            .map(|record| record.state);
        match json {
            None => Ok(None),
            Some(json) => match json.get("status") {
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
    /// resume path under the given tenant scope. Shared by `dispatch_start`,
    /// `dispatch_resume`, and `dispatch_restart` — the three commands converge
    /// on the same engine entry today because the engine does not yet
    /// distinguish a `restart-from-input` rewind from a normal resume (true
    /// rewind requires durable output purge — tracked separately).
    async fn drive(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        match self.engine.resume_execution(scope, execution_id).await {
            Ok(_) => Ok(()),
            // Concurrent dispatcher already holds the lease — the canonical
            // idempotency outcome. Returning `Ok()` here prevents
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
                if let Ok(Some(status)) = self.read_status(scope, execution_id).await
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
    async fn dispatch_start(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        match self.read_status(scope, execution_id).await? {
            None => Err(ControlDispatchError::Rejected(format!(
                "execution {execution_id} not found — start command orphaned"
            ))),
            // Already past the Created gate: either the engine is driving it
            // (Running / Cancelling) or it has already reached a terminal
            // outcome. Re-delivered Start is a no-op.
            Some(
                ExecutionStatus::Running
                | ExecutionStatus::Cancelling
                | ExecutionStatus::Completed
                | ExecutionStatus::Failed
                | ExecutionStatus::Cancelled
                | ExecutionStatus::TimedOut,
            ) => Ok(()),
            Some(ExecutionStatus::Created | ExecutionStatus::Paused) => {
                self.drive(scope, execution_id).await
            },
        }
    }

    async fn dispatch_resume(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        match self.read_status(scope, execution_id).await? {
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
            // `Created`: no signal-driven waits exist yet — drive directly.
            Some(ExecutionStatus::Created) => self.drive(scope, execution_id).await,
            // `Paused`: the execution is suspended awaiting an external signal.
            // Satisfy all signal-driven waits (Waiting{next_attempt_at==None}→Completed)
            // via durable CAS BEFORE re-driving. This is the only code path that
            // calls `satisfy_signal_waits`; Start / Restart / worker re-drives do not,
            // so a crashed-and-reclaimed Paused execution re-parks its wait nodes
            // rather than auto-completing them — the structural discriminator that
            // prevents an unintended auto-approval on crash recovery.
            //
            // `satisfy_signal_waits` now holds the execution lease for its CAS, so
            // errors split into two classes with different ack semantics:
            //
            // - `Leased` → another runner holds the lease and is actively driving this
            //   execution. The control-queue row must NOT be acked — returning
            //   `ControlDispatchError::Deferred` leaves the row in `Processing` so the
            //   B1 reclaim sweep redelivers it once the lease expires.
            //
            // - `CasConflict` / other → a concurrent actor completed or cancelled the
            //   execution between our status read and the CAS. Idempotency applies: ack
            //   the row (`Ok(())`) because the concurrent actor owns the terminal
            //   transition and we must not redeliver.
            Some(ExecutionStatus::Paused) => {
                match self.engine.satisfy_signal_waits(scope, execution_id).await {
                    Ok(SatisfyOutcome::Satisfied(satisfied_count)) => {
                        tracing::info!(
                            %execution_id,
                            satisfied_count,
                            "dispatch_resume: signal waits satisfied; driving execution"
                        );
                    },
                    Ok(SatisfyOutcome::NothingToSatisfy) => {
                        tracing::info!(
                            %execution_id,
                            "dispatch_resume: no signal-driven wait nodes found (already satisfied \
                             or execution has no wait nodes); driving execution"
                        );
                    },
                    Err(EngineError::Leased { ref holder, .. }) => {
                        // Transient lease contention — another runner is actively
                        // driving this execution. Leave the control-queue row in
                        // `Processing` for B1 reclaim to redeliver.
                        //
                        // Observable: typed `ResumeDeferred` event + `tracing::warn`
                        // let operators distinguish expected transient contention (low
                        // rate) from systematic drops due to a routing bug (high rate).
                        tracing::warn!(
                            %execution_id,
                            %holder,
                            "dispatch_resume: satisfy_signal_waits deferred — execution lease \
                             held by another runner; leaving control-queue row for B1 reclaim"
                        );
                        self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                            execution_id,
                            reason: format!("lease held by {holder}"),
                        });
                        return Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} lease held by {holder}; \
                             Resume deferred for B1 reclaim"
                        )));
                    },
                    Err(e) => {
                        // CAS conflict or internal error: a concurrent actor
                        // concurrently resumed or cancelled the execution. The
                        // idempotency contract treats this as a no-op — the
                        // concurrent actor owns the terminal transition. Ack the row
                        // so redelivery does not occur on an already-resolved state.
                        //
                        // Observable: typed `ResumeDeferred` event + `tracing::warn`.
                        tracing::warn!(
                            %execution_id,
                            error = %e,
                            "dispatch_resume: satisfy_signal_waits rejected by CAS conflict; \
                             treating as already-satisfied (concurrent actor owns transition)"
                        );
                        self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                            execution_id,
                            reason: e.to_string(),
                        });
                        return Ok(());
                    },
                }
                self.drive(scope, execution_id).await
            },
        }
    }

    async fn dispatch_restart(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        // rewind-from-input restart requires durable output purge plus a
        // restart counter — neither exists yet. For A2, treat restart as a
        // re-entrant drive of the engine's resume path and honor the same
        // terminal / running idempotency outcomes.
        //
        // Restart-of-terminal intentionally errors so operators see the gap
        // in the `execution_control_queue.error_message` rather than the
        // command silently succeeding and not actually restarting anything.
        match self.read_status(scope, execution_id).await? {
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
                 requires durable output purge — not yet implemented follow-up"
            ))),
            Some(ExecutionStatus::Created | ExecutionStatus::Paused) => {
                self.drive(scope, execution_id).await
            },
        }
    }

    async fn dispatch_cancel(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        // A3 — every non-orphan `Cancel` signals the engine's
        // cancel registry, regardless of the persisted status.
        //
        // The API handler's `cancel_execution` writes the row to `Cancelled`
        // in the same logical operation as the enqueue (control-queue wiring
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
        // at-least-once redelivery.
        match self.read_status(scope, execution_id).await? {
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
                "control-queue: Cancel dispatched — signalled local runner={signalled}"
                               );
                Ok(())
            },
        }
    }

    async fn dispatch_terminate(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        // names `Terminate` "forced termination", but there is no
        // distinct forced-shutdown path in the engine today — the frontier
        // loop aborts in-flight `JoinSet` tasks via the same cooperative
        // `CancellationToken` that `Cancel` trips. Treating `Terminate` as a
        // synonym for `Cancel` is the honest minimum: the operator-visible
        // contract is identical (in-flight work aborts, state reaches a
        // terminal `Cancelled`), and the capability gap — process-level kill
        // or task-set abort — is tracked separately as a future chip. Do not
        // emit half-implemented forced-abort machinery here (operational honesty).
        //
        // (cancel-registry and cooperative-cancel contract) for
        // the design rationale and the upgrade path to a true forced-shutdown
        // distinction.
        self.dispatch_cancel(scope, execution_id).await
    }
}
