//! Engine-owned [`ControlDispatch`] implementation for every durable command.
//!
//! The [`ControlConsumer`] drains `execution_control_queue` rows and hands each typed command to an
//! implementation of [`ControlDispatch`]. [`EngineControlDispatch`] wires the
//! `Start` / `Resume` / `Restart` paths into the engine so that a POST to
//! `/executions` causes node execution. The durable `Cancel` signal the API's
//! `cancel_execution` handler enqueues now reaches the live frontier loop
//! via [`WorkflowEngine::cancel_execution`].
//!
//! ## Idempotency contract
//!
//! Control-queue delivery is at-least-once: the ack path on `mark_completed`
//! may fail after a successful dispatch, and the queue reclaim path will
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
//! - **Resume** splits by persisted status. For a `Paused` execution it calls
//!   `satisfy_signal_waits` before re-driving (the no-live-runner path). Because
//!   `satisfy_signal_waits` holds the execution lease for its CAS, errors split by effect:
//!   `Leased` returns `Deferred` (queue reclaim redelivers); any other error (CAS conflict,
//!   checkpoint failure) re-reads the persisted status — terminal / `Cancelling` → ack; still
//!   non-terminal → `Deferred`. This ensures the Resume is never silently dropped when the
//!   satisfy did not durably land. For a `Running` execution (a signal wait parked with a
//!   `timeout`, so the row never reached `Paused`) it delivers the Resume to the live frontier
//!   loop's resume channel via `WorkflowEngine::resume_live` and gates the ack on the loop's
//!   durable self-arm: the loop checkpoints the arm under its own lease and replies
//!   with the outcome. The row is acked only when the arm durably landed (`Armed`) or there was
//!   nothing to arm (`NothingToArm`); a failed arm checkpoint, a gone loop, or an ack timeout →
//!   `Deferred` for queue reclaim, each with a distinct `ResumeDeferred` reason. No live entry on
//!   this runner (`NoLiveEntry` — a crashed parking runner with a TTL-expired lease, or
//!   cross-runner) is recovered via `recover_running_resume`: the execution lease is the
//!   dead-vs-live oracle — `satisfy_running_signal_waits` arms the matching wait under a
//!   free/expired lease and re-drives, or `Deferred`s when a live owner holds the lease elsewhere;
//!   a recovery against an absent/corrupt row is ack-dropped (no forever-redelivery). Redelivery
//!   is idempotent.
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
use nebula_storage_port::{Scope, StorageError, dto::ResumeTarget, store::ExecutionStore};

use crate::{
    WorkflowEngine,
    control_consumer::{ControlDispatch, ControlDispatchError},
    engine::{CancelDanglingOutcome, ResumeDelivery, ResumeOutcome, SatisfyOutcome},
    error::EngineError,
    event::ExecutionEvent,
};

/// Three-way discriminant returned by [`EngineControlDispatch::read_status_discriminated`].
///
/// Separates the two PERMANENT outcomes (no row, corrupt row) from a transient
/// backend read error so callers can ack-drop permanent states and defer
/// (redeliver) on transient store failures instead of silently losing a valid
/// command.
#[derive(Debug)]
enum StatusRead {
    /// The execution row exists and its `status` field deserialised cleanly.
    Present(ExecutionStatus),
    /// No row was found for this `execution_id` in this `scope` (permanent).
    Absent,
    /// The row exists but its state is unreadable: the `status` field is
    /// missing or failed to deserialise (permanent corruption).
    Corrupt,
}

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
    control_start: ControlStartOwner,
}

#[derive(Clone)]
struct ControlStartOwner {
    handoff: Arc<dyn nebula_storage_port::store::ExecutionTurnHandoff>,
    holder: String,
    lease_ttl: std::time::Duration,
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
    pub fn new(
        engine: Arc<WorkflowEngine>,
        execution: Arc<dyn ExecutionStore>,
        handoff: Arc<dyn nebula_storage_port::store::ExecutionTurnHandoff>,
        holder: String,
        lease_ttl: std::time::Duration,
    ) -> Self {
        Self {
            engine,
            execution,
            control_start: ControlStartOwner {
                handoff,
                holder,
                lease_ttl,
            },
        }
    }

    async fn dispatch_owned_control(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        owner: &ControlStartOwner,
        claim: nebula_storage_port::store::ControlClaimToken,
        command: nebula_storage_port::store::ControlTurnCommand,
    ) -> crate::ClaimedControlDispatchOutcome {
        use crate::{
            ClaimedControlDispatchOutcome as Dispatch, ClaimedControlTurnOutcome as Owner,
        };
        use nebula_error::Classify as _;
        let restart = matches!(
            command,
            nebula_storage_port::store::ControlTurnCommand::Restart
        );
        match self.read_status_discriminated(scope, execution_id).await {
            Ok(StatusRead::Present(status)) if status.is_terminal() => {
                return Dispatch::NotAccepted(if restart {
                    Err(ControlDispatchError::Rejected(
                        "terminal execution restart requires unsupported rewind semantics"
                            .to_owned(),
                    ))
                } else {
                    Ok(())
                });
            },
            Ok(StatusRead::Present(ExecutionStatus::Cancelling)) => {
                return Dispatch::NotAccepted(Ok(()));
            },
            Ok(StatusRead::Absent | StatusRead::Corrupt) => {
                return Dispatch::NotAccepted(if restart {
                    Err(ControlDispatchError::Rejected(
                        "control Restart has no valid execution".to_owned(),
                    ))
                } else {
                    // Preserve the existing moot Resume policy for orphaned
                    // callbacks; this does not dispatch any action.
                    Ok(())
                });
            },
            Err(error) => return Dispatch::NotAccepted(Err(error)),
            Ok(StatusRead::Present(_)) => {},
        }
        match self
            .engine
            .resume_claimed_control_turn(
                scope,
                execution_id,
                crate::ClaimedControlTurnRequest {
                    claim,
                    handoff: Arc::clone(&owner.handoff),
                    command,
                },
            )
            .await
        {
            Owner::ClaimSuperseded => Dispatch::ClaimSuperseded,
            Owner::AcceptanceUnknown(error) => {
                Dispatch::AcceptanceUnknown(ControlDispatchError::Deferred(error.to_string()))
            },
            Owner::Accepted(result) => Dispatch::Accepted(result.map_err(|error| {
                ControlDispatchError::Internal(format!(
                    "accepted execution {execution_id}: {error}"
                ))
            })),
            Owner::NotAccepted(error) => {
                let permanent = matches!(
                    error.category(),
                    nebula_error::ErrorCategory::Validation | nebula_error::ErrorCategory::NotFound
                );
                Dispatch::NotAccepted(Err(if permanent {
                    ControlDispatchError::Rejected(error.to_string())
                } else {
                    ControlDispatchError::Deferred(error.to_string())
                }))
            },
        }
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

    /// Like [`Self::read_status`] but returns a [`StatusRead`] discriminant
    /// instead of collapsing all failures into `Err`.
    ///
    /// The returned `Err` means a transient backend failure such as a
    /// connection or lock failure. The caller defers the command so queue
    /// reclaim can redeliver it after the store recovers.
    ///
    /// `Ok(StatusRead::Absent)` and `Ok(StatusRead::Corrupt)` are PERMANENT:
    /// no amount of redelivery will change the outcome. Ack-dropping them is
    /// safe (and correct — it prevents forever-redelivery on a moot command).
    ///
    async fn read_status_discriminated(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<StatusRead, ControlDispatchError> {
        let record = match self.execution.get(scope, &execution_id.to_string()).await {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(StatusRead::Absent),
            Err(error) => return Self::classify_status_read_error(execution_id, error),
        };
        match record.state.get("status") {
            Some(s) => match serde_json::from_value::<ExecutionStatus>(s.clone()) {
                Ok(status) => Ok(StatusRead::Present(status)),
                Err(_) => Ok(StatusRead::Corrupt),
            },
            None => Ok(StatusRead::Corrupt),
        }
    }

    fn classify_status_read_error(
        execution_id: ExecutionId,
        error: StorageError,
    ) -> Result<StatusRead, ControlDispatchError> {
        match error {
            error
            @ (StorageError::Serialization(_) | StorageError::UnknownSchemaVersion { .. }) => {
                tracing::error!(
                    %execution_id,
                    error = %error,
                    "persisted execution status cannot be decoded"
                );
                Ok(StatusRead::Corrupt)
            },
            error => Err(ControlDispatchError::Deferred(format!(
                "execution {execution_id}: transient store read error during control dispatch: \
                 {error}"
            ))),
        }
    }

    /// Emit a typed [`ExecutionEvent::ResumeDeferred`], log a warning, and
    /// return [`ControlDispatchError::Deferred`] for a `Running` execution
    /// whose live-frontier Resume did not durably arm.
    ///
    /// Centralises the not-durable arm of [`Self::dispatch_resume`]'s
    /// `Running` branch so each cause (arm-checkpoint failed, loop gone, ack
    /// timeout, no live entry) carries a distinct, observable `reason` while
    /// the row stays un-acked in `Processing` for queue reclaim. Deferring is
    /// always safe: Resume redelivery is idempotent.
    fn defer_running_resume(
        engine: &WorkflowEngine,
        execution_id: ExecutionId,
        reason: &str,
    ) -> Result<(), ControlDispatchError> {
        tracing::warn!(
            %execution_id,
            reason,
            "dispatch_resume: live-frontier Resume not durably armed; deferring for queue reclaim"
        );
        engine.emit_event(ExecutionEvent::ResumeDeferred {
            execution_id,
            reason: reason.to_owned(),
        });
        Err(ControlDispatchError::Deferred(format!(
            "execution {execution_id} Resume deferred for queue reclaim: {reason}"
        )))
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
            Err(EngineError::Effect(error)) if error.is_deferred() => Err(
                ControlDispatchError::Deferred(format!("execution {execution_id}: {error}")),
            ),
            Err(EngineError::ContractBundleRead { .. }) => Err(ControlDispatchError::Deferred(
                format!("execution {execution_id}: persisted contract temporarily unavailable"),
            )),
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
    /// leaves the row in `Processing` for the queue reclaim sweep to redeliver
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
                         pending) for queue reclaim"
                    );
                    self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                        execution_id,
                        reason: format!(
                            "post-satisfy drive did not complete ({e}); status={status_desc}; \
                                 armed wait deferred for queue reclaim"
                        ),
                    });
                    Err(ControlDispatchError::Deferred(format!(
                        "execution {execution_id}: post-satisfy drive did not complete ({e}); \
                             status={status_desc}; armed wait deferred for queue reclaim"
                    )))
                },
            },
        }
    }

    /// Recover a no-live-owner `Running` execution whose `Resume` reached
    /// [`Self::dispatch_resume`]'s `NoLiveEntry` arm.
    ///
    /// `NoLiveEntry` means the live-frontier resume channel found no
    /// `RunningEntry` on this runner — either the parking runner crashed with a
    /// now-TTL-expired lease, or the Resume landed on a different runner than
    /// the (possibly still live) owner. [`WorkflowEngine::satisfy_running_signal_waits`]
    /// uses the execution lease as the dead-vs-live oracle: it acquires the
    /// lease (free / TTL-expired ⇒ crashed, no owner) and arms the matching
    /// signal wait(s) under it, or returns [`EngineError::Leased`] (a real owner
    /// is alive elsewhere ⇒ defer). On a successful arm we re-drive via
    /// [`Self::drive_armed_resume`], which completes the armed wait and
    /// which itself Defers on a non-terminal drive (keeping the Resume
    /// redeliverable when the lease holder is a still-crashed runner).
    ///
    /// Outcome → control-queue ack mapping:
    ///
    /// - `Satisfied` (recovered) → `drive_armed_resume` (ack on terminal,
    ///   `Deferred` on a non-terminal drive).
    /// - `NothingToSatisfy` → ack: no matching parked wait to recover (already
    ///   armed / completed / a different node) — an idempotent no-op.
    /// - `ExecutionNotResumable` → ack: a concurrent cancel/terminate moved the
    ///   execution off a resumable status under the lease; the Resume is moot.
    /// - `Leased` → `Deferred`: a live owner elsewhere, so queue reclaim redelivers.
    /// - any other error → re-read the persisted status:
    ///   - row missing (`Ok(None)`) → **ack-drop**: a forged / garbage / corrupt
    ///     id, or a row deleted mid-recovery, is moot. Acking (not `Deferred`)
    ///     closes the only forever-redeliver leak (the row would otherwise cycle
    ///     through queue reclaim forever against a non-existent execution).
    ///   - status unreadable / unparseable (`Err`) → **ack-drop** for the same
    ///     reason: a corrupt row can never be recovered, so redelivering it
    ///     forever is pure churn.
    ///   - terminal / `Cancelling` → ack: a concurrent actor owns the outcome.
    ///   - still non-terminal → `Deferred`: the wait may still be pending; queue
    ///     reclaim redelivers.
    async fn recover_running_resume(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        resume_target: Option<ResumeTarget>,
    ) -> Result<(), ControlDispatchError> {
        match self
            .engine
            .satisfy_running_signal_waits(scope, execution_id, resume_target.as_ref())
            .await
        {
            Ok(SatisfyOutcome::Satisfied(recovered_count)) => {
                tracing::info!(
                    %execution_id,
                    recovered_count,
                    "dispatch_resume: no-live-owner recovery armed the signal wait(s) under a \
                     dead/expired lease; driving the recovered execution"
                );
                // This drive completes the armed wait; a non-terminal outcome keeps
                // the Resume available for queue reclaim.
                self.drive_armed_resume(scope, execution_id).await
            },
            Ok(SatisfyOutcome::NothingToSatisfy) => {
                tracing::info!(
                    %execution_id,
                    "dispatch_resume: no-live-owner recovery found no matching parked signal \
                     wait to arm (already armed / completed / different node); acking as \
                     idempotent no-op"
                );
                Ok(())
            },
            Ok(SatisfyOutcome::ExecutionNotResumable) => {
                tracing::info!(
                    %execution_id,
                    "dispatch_resume: execution left a resumable status before no-live-owner \
                     recovery could arm (concurrent cancel/terminate); acking Resume as \
                     idempotent no-op"
                );
                Ok(())
            },
            Err(EngineError::Leased { ref holder, .. }) => {
                // A live owner holds the lease elsewhere — this is NOT a crashed
                // runner. Do not double-drive; defer so queue reclaim redelivers
                // once the lease frees (or the owner's own resume channel
                // handles it). Mirrors the `Paused` Leased arm.
                // Distinct, machine-greppable target so an operator can alert on
                // a HIGH RATE of recovery deferrals for one `execution_id` — the
                // budget-blind, non-resolving Resume the reclaim exemption made
                // invisible to the bulk sweep counts.
                tracing::warn!(
                    target: "engine::wait::resume_recovery",
                    %execution_id,
                    %holder,
                    "dispatch_resume: no-live-owner recovery deferred — execution lease held \
                     by a live owner elsewhere; leaving control-queue row for queue reclaim"
                );
                self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                    execution_id,
                    reason: format!("recovery deferred: lease held by live owner {holder}"),
                });
                Err(ControlDispatchError::Deferred(format!(
                    "execution {execution_id} no-live-owner recovery: lease held by live owner \
                     {holder}; Resume deferred for queue reclaim"
                )))
            },
            Err(e) => match self.read_status_discriminated(scope, execution_id).await {
                // Row missing: a forged / garbage / corrupt id, or a row deleted
                // mid-recovery. Ack-DROP (not Deferred) so the row is consumed
                // rather than redelivered forever against a non-existent
                // execution — the only forever-redeliver leak this path closes.
                Ok(StatusRead::Absent) => {
                    tracing::warn!(
                        %execution_id,
                        recovery_error = %e,
                        "dispatch_resume: no-live-owner recovery failed and the execution row \
                         is absent (forged/garbage id or deleted mid-recovery); ack-dropping \
                         the Resume to avoid forever-redelivery"
                    );
                    Ok(())
                },
                Ok(StatusRead::Present(status))
                    if status.is_terminal() || matches!(status, ExecutionStatus::Cancelling) =>
                {
                    tracing::info!(
                        %execution_id,
                        %status,
                        recovery_error = %e,
                        "dispatch_resume: no-live-owner recovery did not land but execution is \
                         now {status}; acking as idempotent"
                    );
                    Ok(())
                },
                Ok(StatusRead::Present(status)) => {
                    // Same distinct target as the live-owner defer above: a
                    // sustained rate here flags a non-resolving Resume that the
                    // reclaim budget no longer terminalizes.
                    tracing::warn!(
                        target: "engine::wait::resume_recovery",
                        %execution_id,
                        %status,
                        recovery_error = %e,
                        "dispatch_resume: no-live-owner recovery did not land and execution is \
                         still non-terminal ({status}); deferring Resume for queue reclaim"
                    );
                    self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                        execution_id,
                        reason: format!("recovery did not land ({e}); status={status}"),
                    });
                    Err(ControlDispatchError::Deferred(format!(
                        "execution {execution_id} no-live-owner recovery did not land ({e}); \
                         status={status}; Resume deferred for queue reclaim"
                    )))
                },
                // Permanent corruption — ack-DROP to close the forever-redeliver
                // leak (same as the `Absent` arm above: a corrupt row can never
                // be recovered).
                Ok(StatusRead::Corrupt) => {
                    tracing::warn!(
                        %execution_id,
                        recovery_error = %e,
                        "dispatch_resume: no-live-owner recovery failed and the status re-read \
                         is unreadable/unparseable (corrupt row); ack-dropping the Resume to \
                         avoid forever-redelivery"
                    );
                    Ok(())
                },
                // Transient backend read error on the re-read — the execution may
                // still be valid and non-terminal. Defer so queue reclaim redelivers
                // once the store recovers; do NOT ack-drop a valid Resume.
                Err(deferred) => {
                    tracing::warn!(
                        target: "engine::wait::resume_recovery",
                        %execution_id,
                        recovery_error = %e,
                        re_read_error = %deferred,
                        "dispatch_resume: no-live-owner recovery failed and the status re-read \
                         also hit a transient store error; deferring Resume conservatively for \
                         queue reclaim"
                    );
                    self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                        execution_id,
                        reason: format!(
                            "recovery did not land ({e}); re-read transient error: {deferred}"
                        ),
                    });
                    Err(ControlDispatchError::Deferred(format!(
                        "execution {execution_id} no-live-owner recovery did not land ({e}); \
                         status re-read transient error ({deferred}); Resume deferred \
                         conservatively for queue reclaim"
                    )))
                },
            },
        }
    }
}

#[async_trait]
impl ControlDispatch for EngineControlDispatch {
    async fn dispatch_claimed_resume(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        target: Option<ResumeTarget>,
        claim: nebula_storage_port::store::ControlClaimToken,
    ) -> crate::ClaimedControlDispatchOutcome {
        let owner = &self.control_start;
        self.dispatch_owned_control(
            scope,
            execution_id,
            owner,
            claim,
            nebula_storage_port::store::ControlTurnCommand::Resume { target },
        )
        .await
    }

    async fn dispatch_claimed_restart(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        claim: nebula_storage_port::store::ControlClaimToken,
    ) -> crate::ClaimedControlDispatchOutcome {
        let owner = &self.control_start;
        self.dispatch_owned_control(
            scope,
            execution_id,
            owner,
            claim,
            nebula_storage_port::store::ControlTurnCommand::Restart,
        )
        .await
    }

    async fn dispatch_claimed_start(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        claim: nebula_storage_port::store::ControlClaimToken,
    ) -> crate::ClaimedControlDispatchOutcome {
        use crate::{ClaimedControlDispatchOutcome as Dispatch, ClaimedStartOutcome as Owner};
        let owner = &self.control_start;
        match self.read_status_discriminated(scope, execution_id).await {
            Ok(StatusRead::Present(status))
                if status.is_terminal() || matches!(status, ExecutionStatus::Cancelling) =>
            {
                return Dispatch::NotAccepted(Ok(()));
            },
            Ok(StatusRead::Absent | StatusRead::Corrupt) => {
                return Dispatch::NotAccepted(Err(ControlDispatchError::Rejected(
                    "control Start has no valid execution".to_owned(),
                )));
            },
            Err(error) => return Dispatch::NotAccepted(Err(error)),
            Ok(StatusRead::Present(_)) => {},
        }
        match self
            .engine
            .resume_control_start(
                scope,
                execution_id,
                crate::ClaimedStartRequest {
                    claim,
                    handoff: owner.handoff.as_ref(),
                    holder: &owner.holder,
                    lease_ttl: owner.lease_ttl,
                },
            )
            .await
        {
            Owner::ClaimSuperseded => Dispatch::ClaimSuperseded,
            Owner::AcceptanceUnknown(error) => {
                Dispatch::AcceptanceUnknown(ControlDispatchError::Deferred(error.to_string()))
            },
            Owner::Accepted(result) => Dispatch::Accepted(result.map(|_| ()).map_err(|error| {
                ControlDispatchError::Internal(format!(
                    "accepted execution {execution_id}: {error}"
                ))
            })),
            Owner::NotAccepted(error) => {
                let transient = matches!(
                    error,
                    EngineError::Leased { .. }
                        | EngineError::CasConflict { .. }
                        | EngineError::ControlStartHandoff { .. }
                        | EngineError::ControlStartVersionConflict { .. }
                        | EngineError::ExecutionRead { .. }
                        | EngineError::ContractBundleRead { .. }
                ) || matches!(&error, EngineError::Effect(effect) if effect.is_deferred())
                    || matches!(&error, EngineError::ExactRevision { source }
                    if matches!(source.as_ref(), crate::PlanFlavorRevisionBridgeError::Catalog {
                        source: nebula_storage_port::RevisionCatalogError::Unavailable
                            | nebula_storage_port::RevisionCatalogError::OutcomeUnknown
                    }));
                Dispatch::NotAccepted(Err(if transient {
                    ControlDispatchError::Deferred(error.to_string())
                } else {
                    ControlDispatchError::Rejected(error.to_string())
                }))
            },
        }
    }

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
        resume_target: Option<ResumeTarget>,
    ) -> Result<(), ControlDispatchError> {
        // A Resume is attacker-facing (a webhook/approval callback) and does no
        // work of its own. Unlike Start/Cancel/Restart — whose orphan is a
        // producer bug surfaced as `Rejected` (→ Failed) — a Resume to an
        // unknown / unreadable execution is MOOT: a forged, garbage, or corrupt
        // id can never resume anything. Ack-DROP it (`Ok(())`, the row is
        // consumed) rather than `Rejected` (which would record a noisy Failed
        // row) or any error that redelivers — this closes the only
        // forever-redeliver leak on the Resume path.
        //
        // IMPORTANT: only `StatusRead::Absent` (no row) and `StatusRead::Corrupt`
        // (permanent bad state) are ack-dropped. A transient backend read error
        // (`Err`) propagates as `Deferred` — a store blip during a VALID Resume
        // must never be silently discarded.
        let status = match self.read_status_discriminated(scope, execution_id).await {
            Ok(StatusRead::Present(status)) => status,
            Ok(StatusRead::Absent) => {
                tracing::warn!(
                    %execution_id,
                    "dispatch_resume: execution not found (forged/garbage id or pruned row); \
                     ack-dropping the moot Resume"
                );
                return Ok(());
            },
            Ok(StatusRead::Corrupt) => {
                tracing::warn!(
                    %execution_id,
                    "dispatch_resume: execution status unreadable/unparseable (corrupt row); \
                     ack-dropping the moot Resume to avoid forever-redelivery"
                );
                return Ok(());
            },
            Err(deferred) => {
                // Transient backend read error — the execution may be VALID.
                // Defer so queue reclaim redelivers once the store recovers.
                tracing::warn!(
                    %execution_id,
                    deferred_reason = %deferred,
                    "dispatch_resume: transient store error reading execution status; \
                     deferring Resume for queue reclaim (not ack-dropping — may be a valid Resume)"
                );
                self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                    execution_id,
                    reason: format!("transient store read error: {deferred}"),
                });
                return Err(deferred);
            },
        };
        match status {
            // `Running`: a signal wait parked with a `timeout` keeps the row
            // `Running` with a live frontier loop on the timeout timer.
            // The durable satisfy-CAS path cannot be used here — it would
            // acquire the lease the live loop already holds. Instead deliver
            // the Resume to the live loop's resume channel; the loop self-arms
            // its signal wait under its own lease, DURABLY checkpoints the arm,
            // and replies with the outcome.
            //
            // The control-queue row is acknowledged only when the
            // self-arm checkpoint durably landed (`Armed`) or there was nothing
            // to arm (`NothingToArm` — an idempotent duplicate). Every
            // not-durable outcome — the arm checkpoint failed (the loop lost its
            // lease mid-iteration), the loop exited before replying, the ack
            // timed out, or no live loop exists on this runner — leaves the row
            // un-acked (`Deferred`) for queue reclaim, with a distinct
            // `ResumeDeferred` reason so the cause is observable. Deferring is
            // always safe: Resume redelivery is idempotent.
            //
            // No-live-owner recovery (`NoLiveEntry`): a `Running` execution
            // with no live loop on THIS runner is recovered via
            // `recover_running_resume` — the execution lease is the
            // dead-vs-live oracle (a free/TTL-expired lease ⇒ the parking
            // runner crashed ⇒ arm under the dead lease and re-drive; a live
            // lease elsewhere ⇒ a real owner is driving ⇒ defer). Only a
            // genuine Resume reaches this arm and recovers; a plain
            // crash-recovery re-drive (worker sink / `dispatch_start` /
            // `dispatch_restart`) re-enters `resume_execution` WITHOUT arming,
            // so it re-parks rather than auto-completing.
            ExecutionStatus::Running => {
                match self
                    .engine
                    .resume_live(execution_id, resume_target.clone())
                    .await
                {
                    ResumeDelivery::Acked(ResumeOutcome::Armed { count }) => {
                        tracing::info!(
                            %execution_id,
                            armed_count = count,
                            "dispatch_resume: live frontier durably armed the signal wait(s)"
                        );
                        Ok(())
                    },
                    ResumeDelivery::Acked(ResumeOutcome::NothingToArm) => {
                        tracing::info!(
                            %execution_id,
                            "dispatch_resume: live frontier had no signal-Waiting node to arm \
                             (already armed / none parked); acking as idempotent no-op"
                        );
                        Ok(())
                    },
                    ResumeDelivery::Acked(ResumeOutcome::ArmFailed) => Self::defer_running_resume(
                        &self.engine,
                        execution_id,
                        "live frontier self-arm checkpoint failed (lease lost mid-iteration)",
                    ),
                    ResumeDelivery::Acked(ResumeOutcome::Claimed(_)) => Self::defer_running_resume(
                        &self.engine,
                        execution_id,
                        "claimed control response arrived on the technical Resume path",
                    ),
                    ResumeDelivery::LoopGone => Self::defer_running_resume(
                        &self.engine,
                        execution_id,
                        "live frontier loop exited before confirming the self-arm",
                    ),
                    ResumeDelivery::AckTimeout => Self::defer_running_resume(
                        &self.engine,
                        execution_id,
                        "live frontier did not confirm the self-arm within the ack timeout",
                    ),
                    ResumeDelivery::NoLiveEntry => {
                        // No live loop on this runner. Recover via the lease
                        // dead-vs-live oracle (crashed parking runner with a
                        // TTL-expired lease, OR cross-runner) instead of an
                        // unconditional defer.
                        self.recover_running_resume(scope, execution_id, resume_target)
                            .await
                    },
                }
            },
            ExecutionStatus::Cancelling
            | ExecutionStatus::Completed
            | ExecutionStatus::Failed
            | ExecutionStatus::Cancelled
            | ExecutionStatus::TimedOut => Ok(()),
            // `Created`: no signal-driven waits exist yet — drive directly.
            ExecutionStatus::Created => self.drive(scope, execution_id).await,
            // `Paused`: the execution is suspended awaiting an external signal.
            // Arm all signal-driven waits (Waiting{next_attempt_at == None}
            // → Waiting{next_attempt_at = now}) via durable CAS BEFORE re-driving;
            // The next drive completes each armed wait through port-aware edge
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
            //   Queue reclaim redelivers it once the lease expires.
            //
            // - Any other error (`CasConflict` / `CheckpointFailed` / etc.) → the
            //   satisfy did NOT durably land.  The correct action depends on the
            //   *current* persisted status (re-read after the error):
            //   · terminal / Cancelling → concurrent actor owns the transition → ack.
            //   · still non-terminal (Paused / Running) → the wait may still be
            //     pending → `Deferred` so queue reclaim redelivers the Resume.
            //   Acking unconditionally here would permanently drop the Resume when a
            //   lease TTL-expiry causes a FencedOut (surfaced as CasConflict) while
            //   the execution is still Paused — bounded lost-Resume.
            ExecutionStatus::Paused => {
                match self
                    .engine
                    .satisfy_signal_waits(scope, execution_id, resume_target.as_ref())
                    .await
                {
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
                        // `Processing` for queue reclaim to redeliver.
                        //
                        // Observable: typed `ResumeDeferred` event + `tracing::warn`
                        // let operators distinguish expected transient contention (low
                        // rate) from systematic drops due to a routing bug (high rate).
                        tracing::warn!(
                            %execution_id,
                            %holder,
                            "dispatch_resume: satisfy_signal_waits deferred — execution lease \
                             held by another runner; leaving control-queue row for queue reclaim"
                        );
                        self.engine.emit_event(ExecutionEvent::ResumeDeferred {
                            execution_id,
                            reason: format!("lease held by {holder}"),
                        });
                        return Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} lease held by {holder}; \
                             Resume deferred for queue reclaim"
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
                                // Defer so queue reclaim redelivers the Resume.
                                let status_desc = status
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| "not found".to_owned());
                                tracing::warn!(
                                    %execution_id,
                                    %status_desc,
                                    satisfy_error = %e,
                                    "dispatch_resume: satisfy_signal_waits did not land and \
                                     execution is still non-terminal ({status_desc}); \
                                     deferring Resume for queue reclaim"
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
                                     Resume deferred for queue reclaim"
                                )));
                            },
                            Err(read_err) => {
                                // Status re-read itself failed — conservative: Defer so
                                // Queue reclaim redelivers; don't ack an unverified state.
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
                                     Resume deferred conservatively for queue reclaim"
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
                // Lease-guarded: a held lease (acquire fails ⇒ TTL not expired)
                // means another runner is alive and owns this execution. That
                // owner observes the API's durable `Cancelled` write via its next
                // checkpoint CAS and tears its own frontier down — so we ACK
                // (below) rather than Defer. Deferring would churn the
                // control-queue row through untargeted, budget-capped queue reclaim
                // (which cannot route to the lease holder) until it is marked
                // failed, while never delivering anything to the owner. A
                // genuinely no-live-runner Paused execution has a FREE lease, so
                // `cancel_dangling_nodes` acquires it and terminalizes there.
                match self.engine.cancel_dangling_nodes(scope, execution_id).await {
                    Ok(
                        CancelDanglingOutcome::Cancelled(_)
                        | CancelDanglingOutcome::NothingToCancel,
                    ) => Ok(()),
                    Err(EngineError::Leased { ref holder, .. }) => {
                        // A live owner on another runner holds the lease, and
                        // this process cannot reach its cancel registry.
                        //
                        // Acking here used to be safe because the API wrote
                        // `Cancelled` durably before enqueuing, so the holder
                        // saw the cancel on its next checkpoint CAS. Nothing
                        // writes that state ahead of the runtime any more — the
                        // command row *is* the cancel — so an ack now would drop
                        // the request outright.
                        //
                        // Defer instead. Redelivery is untargeted, so the row
                        // can land on the holder itself, where the local signal
                        // works; and the holder's lease is short-lived by
                        // construction (released on graceful stop, expired on
                        // crash), so a later delivery finds it free. If reclaim
                        // exhausts its budget first the row is marked failed with
                        // this reason attached — an undelivered cancel that is
                        // visible beats one that was acked and lost.
                        tracing::warn!(
                            %execution_id,
                            %holder,
                            "dispatch_cancel: execution lease held by a live owner on another \
                             runner; deferring for redelivery rather than acking an undelivered \
                             cancel"
                        );
                        Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} cancel could not be delivered: lease held \
                             by another runner; deferred for redelivery"
                        )))
                    },
                    Err(e) => {
                        // The cleanup did not durably land; the execution is
                        // already `Cancelled` but its parked nodes are still
                        // non-terminal. Defer so queue reclaim retries — never ack a
                        // cleanup whose effect did not land.
                        tracing::warn!(
                            %execution_id,
                            error = %e,
                            "dispatch_cancel: dangling-node cleanup did not land; deferring for \
                             queue reclaim"
                        );
                        Err(ControlDispatchError::Deferred(format!(
                            "execution {execution_id} cancel dangling-node cleanup did not land \
                             ({e}); deferred for queue reclaim"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_read_errors_distinguish_corruption_from_retryable_failures() {
        let execution_id = ExecutionId::new();

        std::assert_matches!(
            EngineControlDispatch::classify_status_read_error(
                execution_id,
                StorageError::Serialization("oversized state".to_owned()),
            ),
            Ok(StatusRead::Corrupt)
        );
        std::assert_matches!(
            EngineControlDispatch::classify_status_read_error(
                execution_id,
                StorageError::UnknownSchemaVersion { found: 2, max: 1 },
            ),
            Ok(StatusRead::Corrupt)
        );
        std::assert_matches!(
            EngineControlDispatch::classify_status_read_error(
                execution_id,
                StorageError::Connection("database unavailable".to_owned()),
            ),
            Err(ControlDispatchError::Deferred(_))
        );
    }
}
