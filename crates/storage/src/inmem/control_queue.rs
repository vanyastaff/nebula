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
        let mut st = self.inner.lock();
        let now = Instant::now();
        let mut claimed = Vec::new();
        // Deterministic order so a bounded batch is stable across calls.
        let mut ids: Vec<[u8; 16]> = st
            .queue
            .iter()
            .filter(|(_, q)| q.status == "Pending")
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        for id in ids.into_iter().take(batch_size as usize) {
            if let Some(q) = st.queue.get_mut(&id) {
                // Mint the generation in the same critical section that flips
                // Pending -> Processing, so no other claimer can observe the
                // row as claimed under a generation that was not minted yet.
                let Some(generation) = q.claim_generation.checked_add(1) else {
                    // Fail closed: a wrapped generation would let a superseded
                    // token match a future claim.
                    return Err(StorageError::Internal(format!(
                        "control_queue claim generation overflowed for row {}",
                        ulid_hex(&id)
                    )));
                };
                q.claim_generation = generation;
                q.status = "Processing".to_string();
                q.processed_by = Some(*processor);
                q.processed_at = Some(now);
                // Surface the post-reclaim count on the delivered message,
                // matching the SQL backends (which read the `reclaim_count`
                // column back into `ControlMsg` on claim). A consumer that
                // re-claims a reclaimed row therefore observes the bumped
                // count — the cross-runner-redeliver invariant relies on it.
                q.msg.reclaim_count = q.reclaim_count;
                claimed.push(ControlClaim {
                    msg: q.msg.clone(),
                    token: ControlClaimToken::new(id, ClaimGeneration::new(generation)),
                });
            }
        }
        Ok(claimed)
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
