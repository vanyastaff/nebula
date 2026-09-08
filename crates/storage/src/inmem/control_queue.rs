//! In-memory `ControlQueue` over the shared execution-store core.
//!
//! Built from [`super::InMemoryExecutionStore::shared`] so a `commit`'s
//! outbox rows are immediately claimable. Ids are typed 16-byte ULIDs
//! (`[u8; 16]`) — there is no UTF-8-of-ULID encoding. `enqueue` carries
//! the tenant `Scope`; `mark_completed`/`mark_failed` are fenced by the
//! claiming processor so a reclaimed-then-stale runner cannot overwrite a
//! newer claim.

use std::time::Duration;

// Same `tokio::time::Instant` clock as `inmem::execution` (the
// `QueuedMsg.processed_at` field originates there): keeps reclaim
// staleness driven by tokio's clock so paused-time tests are
// deterministic and the field types stay consistent.
use tokio::time::Instant;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::ControlMsg;
use nebula_storage_port::store::{
    ClaimGeneration, ControlClaim, ControlClaimToken, ControlQueue, ReclaimOutcome,
};

use super::execution::{QueuedMsg, SharedState};

fn matches_flavor(
    state: &super::execution::State,
    message: &ControlMsg,
    flavor: nebula_core::WorkerFlavorRevisionId,
) -> bool {
    let Some(execution) = state.rows.get(&message.execution_id) else {
        return false;
    };
    if execution.scope != message.scope {
        return false;
    }
    let Ok(execution_id) = message.execution_id.parse() else {
        return false;
    };
    super::plan_flavor_catalog::execution_matches_live_flavor(
        &state.revision_catalog,
        execution_id,
        flavor,
    )
}

/// Format a raw 16-byte ULID as lowercase hex for `StorageError` ids, without
/// the optional `hex` crate the `inmem` module deliberately avoids.
fn ulid_hex(id: &[u8; 16]) -> String {
    id.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Terminalise a claimed row, fenced on `(row id, Processing, generation)`.
///
/// Mirrors the SQL backends: an absent row is [`StorageError::NotFound`], and a
/// present row the token no longer owns is [`StorageError::FencedOut`] with
/// **no state change**. Returning `Ok` for a superseded token would let a
/// consumer whose claim was reclaimed terminalise a command the current owner
/// is still dispatching.
fn acknowledge(
    state: &mut super::execution::State,
    claim: &ControlClaimToken,
    terminal_status: &str,
    error: Option<&str>,
) -> Result<(), StorageError> {
    let id = claim.row_id();
    let Some(queued) = state.queue.get_mut(id) else {
        return Err(StorageError::NotFound {
            entity: "control_queue",
            id: ulid_hex(id),
        });
    };
    if queued.msg.scope != *claim.scope() {
        return Err(StorageError::NotFound {
            entity: "control_queue",
            id: ulid_hex(id),
        });
    }
    if queued.status != "Processing" || queued.claim_generation != claim.generation().get() {
        return Err(StorageError::FencedOut {
            entity: "control_queue",
            id: ulid_hex(id),
        });
    }
    terminal_status.clone_into(&mut queued.status);
    if let Some(error) = error {
        queued.error_message = Some(error.to_owned());
    }
    Ok(())
}

/// In-memory durable-outbox handle. Shares the execution store's core.
#[derive(Debug, Clone)]
pub struct InMemoryControlQueue {
    inner: SharedState,
}

impl InMemoryControlQueue {
    fn claim_filtered(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        worker_flavor: Option<nebula_core::WorkerFlavorRevisionId>,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        let mut state = self.inner.lock();
        let mut ids: Vec<_> = state
            .queue
            .iter()
            .filter(|(_, queued)| {
                queued.status == "Pending"
                    && worker_flavor
                        .is_none_or(|flavor| matches_flavor(&state, &queued.msg, flavor))
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids.truncate(batch_size.clamp(1, 256) as usize);
        for id in &ids {
            if state.queue[id].claim_generation == u64::MAX {
                return Err(StorageError::Internal(format!(
                    "control_queue claim generation overflowed for row {}",
                    ulid_hex(id),
                )));
            }
        }
        let now = Instant::now();
        let mut claimed = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(queued) = state.queue.get_mut(&id) {
                queued.claim_generation += 1;
                "Processing".clone_into(&mut queued.status);
                queued.processed_by = Some(*processor);
                queued.processed_at = Some(now);
                queued.msg.reclaim_count = queued.reclaim_count;
                claimed.push(ControlClaim {
                    msg: queued.msg.clone(),
                    token: ControlClaimToken::new(
                        id,
                        ClaimGeneration::new(queued.claim_generation),
                        queued.msg.scope.clone(),
                    ),
                });
            }
        }
        tracing::debug!(
            exact_flavor = worker_flavor.is_some(),
            claimed = claimed.len(),
            "claimed control commands"
        );
        Ok(claimed)
    }

    /// Build a control queue over an execution store's shared core.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
        }
    }

    /// Non-consuming snapshot of every enqueued row as
    /// `(msg, status)` pairs, ordered by id for determinism.
    ///
    /// This is the port-side structural equivalent of the legacy
    /// `InMemoryControlQueueRepo::snapshot` (test assertions need to see
    /// pending rows *without* the status flip `claim_pending` performs —
    /// e.g. the §13 knife asserts both the `Start` and `Cancel` rows are
    /// still `Pending`). Inspection only; never used on a hot path.
    #[must_use]
    pub fn snapshot(&self) -> Vec<(ControlMsg, String)> {
        let st = self.inner.lock();
        let mut rows: Vec<(&[u8; 16], &QueuedMsg)> = st.queue.iter().collect();
        rows.sort_unstable_by_key(|(id, _)| **id);
        rows.into_iter()
            .map(|(_, q)| {
                // Reflect the live reclaim bookkeeping on the snapshot's
                // message, matching the SQL backends where `reclaim_count`
                // is a row column (a swept-but-not-yet-reclaimed row already
                // shows the bumped count).
                let mut msg = q.msg.clone();
                msg.reclaim_count = q.reclaim_count;
                (msg, q.status.clone())
            })
            .collect()
    }

    /// Test-only detailed snapshot: `(msg, status, error_message)` per
    /// row, ordered by id. The SQL backends expose the `error_message`
    /// column on a failed row; this surfaces the same for in-memory
    /// assertions (e.g. a poison row marked `Failed` with a reason).
    #[doc(hidden)]
    #[must_use]
    pub fn snapshot_detailed(&self) -> Vec<(ControlMsg, String, Option<String>)> {
        let st = self.inner.lock();
        let mut rows: Vec<(&[u8; 16], &QueuedMsg)> = st.queue.iter().collect();
        rows.sort_unstable_by_key(|(id, _)| **id);
        rows.into_iter()
            .map(|(_, q)| {
                let mut msg = q.msg.clone();
                msg.reclaim_count = q.reclaim_count;
                (msg, q.status.clone(), q.error_message.clone())
            })
            .collect()
    }

    /// Test-only seed of an already-`Processing` row owned by a (dead)
    /// `processor`, claimed `stale_for` ago, with a given prior
    /// `reclaim_count`. Reproduces a crashed-runner orphan for reclaim
    /// tests — the legacy `InMemoryControlQueueRepo` allowed enqueuing a
    /// pre-built `Processing` entry; the port queue's `enqueue` is always
    /// `Pending`, so this restores that test affordance structurally.
    #[doc(hidden)]
    pub fn seed_processing(
        &self,
        msg: &ControlMsg,
        processor: [u8; 16],
        stale_for: Duration,
        reclaim_count: u32,
    ) {
        let now = Instant::now();
        let processed_at = now.checked_sub(stale_for).unwrap_or(now);
        let mut st = self.inner.lock();
        st.queue.insert(
            msg.id,
            QueuedMsg {
                msg: msg.clone(),
                status: "Processing".to_string(),
                processed_by: Some(processor),
                processed_at: Some(processed_at),
                reclaim_count,
                error_message: None,
                claim_generation: 0,
            },
        );
    }
}

#[async_trait::async_trait]
impl ControlQueue for InMemoryControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        if st.queue.contains_key(&msg.id) {
            return Err(StorageError::Duplicate {
                entity: "control_queue",
                detail: ulid_hex(&msg.id),
            });
        }
        st.queue.insert(
            msg.id,
            QueuedMsg {
                msg: msg.clone(),
                status: "Pending".to_string(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );
        tracing::debug!(
            target: "nebula_storage::inmem",
            command = msg.command.as_str(),
            "control_queue: enqueued"
        );
        Ok(())
    }

    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        self.claim_filtered(processor, batch_size, None)
    }

    async fn claim_pending_for_flavor(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        worker_flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        self.claim_filtered(processor, batch_size, Some(worker_flavor))
    }

    async fn mark_completed(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Completed", None)
    }

    async fn mark_failed(
        &self,
        claim: &ControlClaimToken,
        error: &str,
    ) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Failed", Some(error))
    }

    async fn release_claim(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Pending", None)?;
        // Clear the claim bookkeeping so the row looks untouched to the next
        // claimer; leaving `processed_at` set would make an immediately
        // re-claimed row look stale to the reclaim sweep.
        if let Some(queued) = st.queue.get_mut(claim.row_id()) {
            queued.processed_by = None;
            queued.processed_at = None;
        }
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        let mut st = self.inner.lock();
        let now = Instant::now();
        let mut outcome = ReclaimOutcome::default();
        for q in st.queue.values_mut() {
            if q.status != "Processing" {
                continue;
            }
            let stale = match q.processed_at {
                Some(at) => now.duration_since(at) >= reclaim_after,
                None => false,
            };
            if !stale {
                continue;
            }
            // A `Resume` row is EXEMPT from the exhaust budget (ADR-0099 W-S3b):
            // a Resume does no work of its own and cannot poison-loop, so the
            // budget must never force-Fail it — engine liveness (`acquire_lease`)
            // and the wait's own timeout are the only terminal authorities. It
            // keeps redelivering past `reclaim_count >= max` (mirrors the SQL
            // backends' `command <> 'Resume'` exhaust guard + `OR command =
            // 'Resume'` redeliver widening) rather than wedging in `Processing`.
            let is_resume = q.msg.command == nebula_storage_port::dto::ControlCommand::Resume;
            if q.reclaim_count >= max_reclaim_count && !is_resume {
                q.status = "Failed".to_string();
                q.error_message = Some(format!(
                    "reclaim exhausted: presumed dead after {} reclaims",
                    q.reclaim_count
                ));
                outcome.exhausted += 1;
            } else {
                q.status = "Pending".to_string();
                q.reclaim_count = q.reclaim_count.saturating_add(1);
                q.processed_by = None;
                q.processed_at = None;
                outcome.reclaimed += 1;
            }
        }
        Ok(outcome)
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // In-memory rows carry monotonic `Instant`s, not wall-clock
        // timestamps, so age-based pruning is a no-op (parity with the
        // legacy in-memory control queue).
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use nebula_storage_port::dto::{ControlCommand, ControlMsg};
    use nebula_storage_port::store::ControlQueue;
    use nebula_storage_port::{Scope, StorageError};

    use super::InMemoryControlQueue;
    use crate::inmem::InMemoryExecutionStore;

    #[tokio::test]
    async fn duplicate_enqueue_preserves_the_active_claim() {
        let store = InMemoryExecutionStore::new();
        let queue = InMemoryControlQueue::new(&store);
        let message = ControlMsg {
            id: [1; 16],
            execution_id: "execution".to_owned(),
            command: ControlCommand::Start,
            scope: Scope::new("workspace", "organization"),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        };
        queue.enqueue(&message).await.unwrap();
        let claim = queue.claim_pending(&[2; 16], 1).await.unwrap().remove(0);

        assert!(matches!(
            queue.enqueue(&message).await,
            Err(StorageError::Duplicate {
                entity: "control_queue",
                ..
            })
        ));
        queue.mark_completed(&claim.token).await.unwrap();
    }
}
