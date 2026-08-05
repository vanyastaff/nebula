//! In-memory dispatch-claim → execution-turn handoff.
//!
//! The queue rows and the execution rows already live under the execution
//! store's single lock, so the whole handoff runs in one critical section —
//! the in-memory equivalent of the SQL backends' single transaction.
//!
//! The mutex is not a transaction: there is no rollback, so every rejection is
//! decided before the first write. Acquiring the lease and then discovering the
//! claim was superseded would leave the execution leased to a worker that must
//! not run it, with no way to undo.

use std::time::Duration;

use nebula_storage_port::store::{ExecutionTurnHandoff, TurnAcceptance, TurnHandoff};
use nebula_storage_port::{FencingToken, StorageError};

use super::execution::SharedState;

/// In-memory owner of the dispatch-claim → execution-turn handoff.
#[derive(Clone, Debug)]
pub struct InMemoryTurnHandoff {
    inner: SharedState,
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
    #[tracing::instrument(
        level = "debug",
        name = "turn_handoff.accept_turn",
        skip(self, handoff),
        fields(
            backend = "in_memory",
            execution_id = handoff.execution_id,
            claim_generation = handoff.claim.generation().get(),
            outcome = tracing::field::Empty,
        )
    )]
    async fn accept_turn(&self, handoff: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError> {
        let ttl = normalized_ttl(handoff.lease_ttl);
        let result = {
            let mut state = self.inner.lock();
            let now = chrono::Utc::now();

            // Decide everything before writing: this critical section cannot
            // roll back, so a rejection discovered after a write would leave
            // the execution leased to a worker that must not run it.
            let job = state
                .jobs
                .get(handoff.claim.row_id())
                .ok_or_else(|| StorageError::not_found("job_dispatch", "claimed row"))?;
            if job.status != "Processing"
                || job.claim_generation != handoff.claim.generation().get()
            {
                Ok(TurnAcceptance::ClaimSuperseded)
            } else {
                let row = state
                    .rows
                    .get(handoff.execution_id)
                    .filter(|row| &row.scope == handoff.scope)
                    .ok_or_else(|| StorageError::not_found("execution", handoff.execution_id))?;
                let held = matches!(row.lease_expires_at, Some(expiry) if expiry >= now);
                if held {
                    // The queue row is deliberately left claimed: acknowledging
                    // here would make it terminal while no owner ever ran the
                    // turn, so the sweep must still be able to redeliver it.
                    Ok(TurnAcceptance::TurnHeldByAnotherOwner)
                } else {
                    // Past this point every write is infallible, so the lease
                    // and the acknowledgement land together or not at all.
                    let generation = {
                        let row = state.rows.get_mut(handoff.execution_id).ok_or_else(|| {
                            StorageError::not_found("execution", handoff.execution_id)
                        })?;
                        // Every acquire bumps the generation, so a token from
                        // before this handoff is dead — including one held by a
                        // crashed-then-restarted runner reusing its identity.
                        row.fencing_generation = row.fencing_generation.saturating_add(1);
                        handoff
                            .holder
                            .clone_into(row.lease_holder.get_or_insert_with(String::new));
                        row.lease_expires_at = Some(
                            now + chrono::Duration::from_std(ttl)
                                .unwrap_or_else(|_overflow| chrono::Duration::zero()),
                        );
                        row.fencing_generation
                    };
                    let job = state
                        .jobs
                        .get_mut(handoff.claim.row_id())
                        .ok_or_else(|| StorageError::not_found("job_dispatch", "claimed row"))?;
                    "Dispatched".clone_into(&mut job.status);
                    Ok(TurnAcceptance::Accepted {
                        fence: FencingToken::from_generation(generation),
                    })
                }
            }
        };

        let outcome = acceptance_label(&result);
        tracing::Span::current().record("outcome", outcome);
        tracing::debug!(
            target: "nebula_storage::inmem",
            outcome,
            "dispatch claim handed off to an execution turn"
        );
        result
    }
}

/// Stable label naming one handoff outcome, so every backend reports the same
/// vocabulary on its spans.
pub(crate) const fn acceptance_label(
    result: &Result<TurnAcceptance, StorageError>,
) -> &'static str {
    match *result {
        Ok(TurnAcceptance::Accepted { .. }) => "accepted",
        Ok(TurnAcceptance::ClaimSuperseded) => "claim_superseded",
        Ok(TurnAcceptance::TurnHeldByAnotherOwner) => "turn_held_by_another_owner",
        Err(_) => "error",
    }
}
