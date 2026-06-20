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
//! - **Start / Restart** short-circuit on persisted status. A command arriving for an
//!   already-terminal execution is `Ok(())`; a command arriving for a `Running` / `Cancelling`
//!   execution is `Ok(())` (a sibling runner already owns the dispatch). A race where a second
//!   dispatcher wins the lease between our read and the engine's own lease acquire surfaces as
//!   [`EngineError::Leased`], which `drive()` maps to `Ok(())` so the same execution is not fenced
//!   as a consumer failure.
//!
//! - **Resume** additionally calls `satisfy_signal_waits` for `Paused` executions before
//!   re-driving. Because `satisfy_signal_waits` holds the execution lease for its CAS, errors
//!   split by effect: `Leased` returns `Deferred` (B1 reclaim redelivers); any other error
//!   (CAS conflict, checkpoint failure) re-reads the persisted status — terminal / `Cancelling`
//!   → ack; still non-terminal → `Deferred`. This ensures the Resume is never silently dropped
//!   when the satisfy did not durably land.
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
    engine::{CancelDanglingOutcome, SatisfyOutcome},
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

    /// Drive an execution whose signal wait `satisfy_signal_waits` has already
    /// **armed** (`next_attempt_at = Some`), keeping the Resume redeliverable
    /// until the armed wait is actually completed.
    ///
    /// Unlike [`Self::drive`], a drive that cannot run to a durable outcome —
    /// lease contention (`EngineError::Leased`), a CAS conflict, or any other
    /// non-terminal error — returns [`ControlDispatchError::Deferred`], NOT
    /// `Ok`. The wait has been durably armed but not completed; acking the
    /// control-queue row here would strand the paused execution when the lease
    /// holder is a crashed/stalled runner whose TTL has not expired yet (it
    /// never completes the armed wait, and no redelivery remains). Deferring
    /// leaves the row in `Processing` for the B1 reclaim sweep to redeliver
    /// once the lease frees, and a later drive completes the armed wait.
    ///
    /// The Resume is acked ONLY when a post-error status re-read shows the
    /// execution is genuinely terminal or `Cancelling` (a concurrent actor owns
    /// the outcome, so the Resume is moot).
    async fn drive_armed_resume(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        match self.engine.resume_execution(scope, execution_id).await {
            Ok(_) => Ok(()),
            Err(e) => match self.read_status(scope, execution_id).await {
                Ok(Some(status))
                    if status.is_terminal() || matches!(status, ExecutionStatus::Cancelling) =>
                {
                    tracing::info!(
                        %execution_id,
                        %status,
                        drive_error = %e,
                        "dispatch_resume: post-satisfy drive did not complete but execution \
                         is now {status}; acking as idempotent"
                    );
                    Ok(())
                },
                other => {
                    let status_desc = match other {
                        Ok(s) => s
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "not found".to_owned()),
                        Err(read_err) => format!("status re-read failed: {read_err}"),
                    };
                    tracing::warn!(
                        %execution_id,
                        %status_desc,
                        drive_error = %e,
                        "dispatch_resume: post-satisfy drive did not complete and execution is \
                         not terminal ({status_desc}); deferring Resume (armed wait still \
                         pending) for B1 reclaim"
                    );
                    self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                        execution_id,
                        reason: format!(
                            "post-satisfy drive did not complete ({e}); status={status_desc}; \
                                 armed wait deferred for B1 reclaim"
                        ),
                    });
                    Err(ControlDispatchError::Deferred(format!(
                        "execution {execution_id}: post-satisfy drive did not complete ({e}); \
                             status={status_desc}; armed wait deferred for B1 reclaim"
                    )))
                },
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
            // Arm all signal-driven waits (Waiting{next_attempt_at == None}
            // → Waiting{next_attempt_at = now}) via durable CAS BEFORE re-driving;
            // Phase-0b then completes each armed wait through PORT-AWARE edge
            // routing (completing it here would route port-blind). This is the
            // only code path that
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
            // - Any other error (`CasConflict` / `CheckpointFailed` / etc.) → the
            //   satisfy did NOT durably land.  The correct action depends on the
            //   *current* persisted status (re-read after the error):
            //   · terminal / Cancelling → concurrent actor owns the transition → ack.
            //   · still non-terminal (Paused / Running) → the wait may still be
            //     pending → `Deferred` so B1 reclaim redelivers the Resume.
            //   Acking unconditionally here would permanently drop the Resume when a
            //   lease TTL-expiry causes a FencedOut (surfaced as CasConflict) while
            //   the execution is still Paused — bounded lost-Resume.
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
                    Ok(SatisfyOutcome::ExecutionNotResumable) => {
                        // A concurrent Cancel/Terminate moved the execution off
                        // `Paused` between our pre-lease status read and the
                        // under-lease reload inside `satisfy_signal_waits`. The
                        // Resume is moot — ack WITHOUT driving, mirroring the
                        // up-front terminal/`Cancelling` handling above. Driving
                        // here would re-enter the engine on a terminating
                        // execution; satisfy already made no durable write.
                        tracing::info!(
                            %execution_id,
                            "dispatch_resume: execution left Paused before satisfy (concurrent \
                             cancel/terminate); acking Resume as idempotent no-op"
                        );
                        return Ok(());
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
                        // CAS conflict / fencing / checkpoint failure: the satisfy
                        // did NOT durably land. The idempotency rule is:
                        //   - ack ONLY when a post-error status re-read confirms the
                        //     execution is now terminal (concurrent actor owns the
                        //     transition) or Cancelling (cancel already in flight).
                        //   - Defer otherwise — the wait may still be pending and the
                        //     Resume must not be lost.
                        //
                        // Rationale: if our lease TTL-expires mid-commit, another runner
                        // acquires the lease, bumps the generation, and our write is
                        // FencedOut (surfaced here as CasConflict) — the wait node is
                        // still Waiting and the execution is still Paused.  Acking here
                        // would permanently drop the Resume (bounded lost-Resume of the
                        // same class as the P1 bug).
                        match self.read_status(scope, execution_id).await {
                            Ok(Some(status))
                                if status.is_terminal()
                                    || matches!(status, ExecutionStatus::Cancelling) =>
                            {
                                // Concurrent actor drove the execution to a genuine
                                // terminal or cancelling state — ack is safe here.
                                tracing::info!(
                                    %execution_id,
                                    %status,
                                    satisfy_error = %e,
                                    "dispatch_resume: satisfy_signal_waits did not land but \
                                     execution is now {status}; acking as idempotent"
                                );
                                return Ok(());
                            },
                            Ok(status) => {
                                // Execution is not yet terminal (Paused / Running / Created
                                // or row missing): the wait may still be pending.
                                // Defer so B1 reclaim redelivers the Resume.
                                let status_desc = status
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| "not found".to_owned());
                                tracing::warn!(
                                    %execution_id,
                                    %status_desc,
                                    satisfy_error = %e,
                                    "dispatch_resume: satisfy_signal_waits did not land and \
                                     execution is still non-terminal ({status_desc}); \
                                     deferring Resume for B1 reclaim"
                                );
                                self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                                    execution_id,
                                    reason: format!(
                                        "satisfy did not land ({e}); status={status_desc}"
                                    ),
                                });
                                return Err(ControlDispatchError::Deferred(format!(
                                    "execution {execution_id}: satisfy_signal_waits did not \
                                     durably commit ({e}); status={status_desc}; \
                                     Resume deferred for B1 reclaim"
                                )));
                            },
                            Err(read_err) => {
                                // Status re-read itself failed — conservative: Defer so
                                // B1 reclaim redelivers; don't ack an unverified state.
                                tracing::warn!(
                                    %execution_id,
                                    satisfy_error = %e,
                                    read_error = %read_err,
                                    "dispatch_resume: satisfy_signal_waits did not land and \
                                     status re-read also failed; deferring Resume conservatively"
                                );
                                self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                                    execution_id,
                                    reason: format!(
                                        "satisfy did not land ({e}); status re-read failed: \
                                         {read_err}"
                                    ),
                                });
                                return Err(ControlDispatchError::Deferred(format!(
                                    "execution {execution_id}: satisfy_signal_waits did not land \
                                     ({e}) and status re-read failed ({read_err}); \
                                     Resume deferred conservatively for B1 reclaim"
                                )));
                            },
                        }
                    },
                }
                // Post-satisfy drive: keep the Resume redeliverable until the
                // armed wait is actually completed (a Leased/CAS-conflict drive
                // must Defer, not ack — see `drive_armed_resume`).
                self.drive_armed_resume(scope, execution_id).await
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
                if signalled {
                    // A live in-process frontier owns this execution: its loop
                    // teardown (`drain_pending_to_cancelled`) terminalizes the
                    // parked/queued nodes. Ack — nothing more to do here.
                    return Ok(());
                }
                // No in-process runner: either a `Paused` (signal-wait)
                // execution with no live frontier, or a cross-runner live
                // execution. Durably terminalize any parked nodes so a
                // `Cancelled` execution never retains a non-terminal node.
                // Lease-guarded: a held lease means a live frontier (this is the
                // cross-runner case) is tearing down or another runner owns it →
                // Defer for B1 reclaim rather than racing its node writes.
                match self.engine.cancel_dangling_nodes(scope, execution_id).await {
                    Ok(
                        CancelDanglingOutcome::Cancelled(_)
                        | CancelDanglingOutcome::NothingToCancel,
                    ) => Ok(()),
                    Ok(CancelDanglingOutcome::StatusNotCancelled) => {
                        // The cancel is not yet durably recorded on the execution
                        // (the API writes `Cancelled` before enqueuing `Cancel`,
                        // so this is a producer-ordering anomaly). Defer rather
                        // than ack-and-drop the node cleanup — B1 reclaim
                        // redelivers until the status reflects the cancel.
                        tracing::warn!(
                            %execution_id,
                            "dispatch_cancel: cancel not yet durably recorded on the execution; \
                             deferring node cleanup for B1 reclaim"
                        );
                        Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} cancel not yet durably recorded; node \
                             cleanup deferred for B1 reclaim"
                        )))
                    },
                    Err(EngineError::Leased { ref holder, .. }) => {
                        tracing::debug!(
                            %execution_id,
                            %holder,
                            "dispatch_cancel: dangling-node cleanup deferred — execution lease \
                             held by another runner (its teardown or B1 reclaim completes the cancel)"
                        );
                        Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} cancel dangling-node cleanup deferred; \
                             lease held by {holder}"
                        )))
                    },
                    Err(e) => {
                        // The cleanup did not durably land; the execution is
                        // already `Cancelled` but its parked nodes are still
                        // non-terminal. Defer so B1 reclaim retries — never ack a
                        // cleanup whose effect did not land.
                        tracing::warn!(
                            %execution_id,
                            error = %e,
                            "dispatch_cancel: dangling-node cleanup did not land; deferring for \
                             B1 reclaim"
                        );
                        Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} cancel dangling-node cleanup did not land \
                             ({e}); deferred for B1 reclaim"
                        )))
                    },
                }
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
