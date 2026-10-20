//! In-memory control-claim → execution-turn handoff.
//!
//! The control-queue rows and the execution rows already live under the execution
//! store's single lock, so the whole handoff runs in one critical section —
//! the in-memory equivalent of the SQL backends' single transaction.
//!
//! The mutex is not a transaction: there is no rollback, so every rejection is
//! decided before the first write. Acquiring the lease and then discovering the
//! claim was superseded would leave the execution leased to a worker that must
//! not run it, with no way to undo.

use std::time::Duration;

use nebula_storage_port::store::{
    ControlStartAcceptance, ControlStartHandoff, ExecutionTurnHandoff, TurnRecovery,
};
use nebula_storage_port::{FencingToken, StorageError};

use super::execution::SharedState;

/// In-memory owner of the control-claim → execution-turn handoff.
#[derive(Clone)]
pub struct InMemoryTurnHandoff {
    inner: SharedState,
    clock: std::sync::Arc<dyn nebula_core::accessor::Clock>,
}

impl std::fmt::Debug for InMemoryTurnHandoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryTurnHandoff")
            .finish_non_exhaustive()
    }
}

impl InMemoryTurnHandoff {
    /// Build a handoff over an execution store's shared core.
    ///
    /// Sharing the core is required, not convenient: the lease write and the
    /// queue acknowledgement must land together, and two stores would give
    /// them two boundaries.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
            clock: store.clock.clone(),
        }
    }
}

/// Clamp the lease TTL the same way the execution store does, so a handoff
/// cannot mint a lease the store itself would have refused to issue.
fn normalized_ttl(ttl: Duration) -> Duration {
    Duration::from_secs_f64(ttl.as_secs_f64().clamp(1.0, 86_400.0))
}

#[async_trait::async_trait]
impl ExecutionTurnHandoff for InMemoryTurnHandoff {
    async fn record_control_flavor_refusal(
        &self,
        request: &nebula_storage_port::store::ControlFlavorRefusal<'_>,
    ) -> Result<nebula_storage_port::store::ControlFlavorRefusalOutcome, StorageError> {
        super::control_turn::record_flavor(&self.inner, self.clock.as_ref(), request)
    }

    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        nebula_storage_port::StorageBackendKind::InMemory
    }

    async fn commit_control_turn(
        &self,
        commit: &nebula_storage_port::store::ControlTurnCommit<'_>,
    ) -> Result<nebula_storage_port::store::ControlTurnCommitOutcome, StorageError> {
        super::control_turn::commit(&self.inner, self.clock.as_ref(), commit)
    }
    #[tracing::instrument(name = "turn_handoff.accept_control_start", skip_all,
        fields(backend = "in_memory", execution_id = handoff.execution_id(),
            outcome = tracing::field::Empty))]
    async fn accept_control_start(
        &self,
        handoff: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError> {
        self.accept_control_start_impl(handoff)
    }
}

#[async_trait::async_trait]
impl TurnRecovery for InMemoryTurnHandoff {
    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        nebula_storage_port::StorageBackendKind::InMemory
    }

    async fn list_recoverable_turns(
        &self,
        flavor: nebula_core::WorkerFlavorRevisionId,
        after: Option<&str>,
        limit: u32,
    ) -> Result<nebula_storage_port::store::RecoverableTurnPage, StorageError> {
        super::turn_recovery::list(&self.inner, self.clock.as_ref(), flavor, after, limit)
    }
    async fn accept_recovery_turn(
        &self,
        handoff: &nebula_storage_port::store::RecoveryTurnHandoff<'_>,
    ) -> Result<nebula_storage_port::store::RecoveryTurnAcceptance, StorageError> {
        super::turn_recovery::accept(&self.inner, self.clock.as_ref(), handoff)
    }
}

impl InMemoryTurnHandoff {
    fn accept_control_start_impl(
        &self,
        handoff: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError> {
        let result = (|| {
            i64::try_from(handoff.claim().generation().get()).map_err(|_| {
                StorageError::Internal("control handoff claim generation is invalid".into())
            })?;
            i64::try_from(handoff.expected_execution_version()).map_err(|_| {
                StorageError::Internal("control handoff execution version is invalid".into())
            })?;
            let mut state = self.inner.lock();
            let now = self.clock.now();
            let valid_claim = state
                .queue
                .get(handoff.claim().row_id())
                .is_some_and(|row| {
                    row.status == "Processing"
                        && row.claim_generation == handoff.claim().generation().get()
                        && row.msg.command == nebula_storage_port::dto::ControlCommand::Start
                        && row.msg.execution_id == handoff.execution_id()
                        && &row.msg.scope == handoff.scope()
                });
            if !valid_claim {
                return Ok(ControlStartAcceptance::ClaimSuperseded);
            }
            let Some(row) = state
                .rows
                .get(handoff.execution_id())
                .filter(|row| &row.scope == handoff.scope())
            else {
                return Ok(ControlStartAcceptance::ClaimSuperseded);
            };
            let Ok(execution_id) = handoff.execution_id().parse() else {
                return Ok(ControlStartAcceptance::ClaimSuperseded);
            };
            if !super::plan_flavor_catalog::execution_matches_live_flavor(
                &state.revision_catalog,
                execution_id,
                handoff.worker_flavor_revision_id(),
            ) {
                return Ok(ControlStartAcceptance::ClaimSuperseded);
            }
            if row.version != handoff.expected_execution_version() {
                return Ok(ControlStartAcceptance::VersionConflict {
                    actual: row.version,
                });
            }
            if row.lease_expires_at.is_some_and(|expiry| expiry >= now) {
                return Ok(ControlStartAcceptance::TurnHeldByAnotherOwner);
            }
            let generation = row
                .fencing_generation
                .checked_add(1)
                .filter(|generation| i64::try_from(*generation).is_ok())
                .ok_or_else(|| {
                    StorageError::Internal("control handoff fence is exhausted".into())
                })?;
            let duration = chrono::Duration::from_std(normalized_ttl(handoff.lease_ttl()))
                .map_err(|_| {
                    StorageError::Internal("control handoff lease duration is invalid".into())
                })?;
            let expires = now.checked_add_signed(duration).ok_or_else(|| {
                StorageError::Internal("control handoff lease deadline is invalid".into())
            })?;
            // The observation is the only fallible write; it lands first so a
            // failure leaves lease, queue and marker untouched.
            let payload = crate::control_turn::accepted_turn_payload(
                nebula_execution::ExecutionControlSource::ControlQueue {
                    row_id: *handoff.claim().row_id(),
                    queue_claim_generation: handoff.claim().generation().get(),
                },
                generation,
                now,
            )?;
            super::control_turn::append_observation(&mut state, handoff.execution_id(), payload)?;
            let super::execution::State { rows, queue, .. } = &mut *state;
            let row = rows.get_mut(handoff.execution_id()).ok_or_else(|| {
                StorageError::Internal("control handoff execution disappeared".into())
            })?;
            let queued = queue.get_mut(handoff.claim().row_id()).ok_or_else(|| {
                StorageError::Internal("control handoff claim disappeared".into())
            })?;
            row.fencing_generation = generation;
            row.lease_holder = Some(handoff.holder().to_owned());
            row.lease_expires_at = Some(expires);
            "Completed".clone_into(&mut queued.status);
            state.accepted_turns.insert(
                handoff.execution_id().to_owned(),
                super::turn_recovery::AcceptedTurn {
                    scope: handoff.scope().clone(),
                    generation,
                    source: "ControlStart",
                    queue_id: *handoff.claim().row_id(),
                },
            );
            Ok(ControlStartAcceptance::Accepted {
                fence: FencingToken::from_generation(generation),
            })
        })();
        tracing::Span::current().record("outcome", control_acceptance_label(&result));
        result
    }
}

pub(crate) const fn control_acceptance_label(
    result: &Result<ControlStartAcceptance, StorageError>,
) -> &'static str {
    match result {
        Ok(ControlStartAcceptance::Accepted { .. }) => "accepted",
        Ok(ControlStartAcceptance::ClaimSuperseded) => "claim_superseded",
        Ok(ControlStartAcceptance::TurnHeldByAnotherOwner) => "turn_held_by_another_owner",
        Ok(ControlStartAcceptance::VersionConflict { .. }) => "version_conflict",
        Ok(_) => "unsupported_outcome",
        Err(StorageError::AcknowledgementUnknown { .. }) => "acknowledgement_unknown",
        Err(_) => "error",
    }
}
