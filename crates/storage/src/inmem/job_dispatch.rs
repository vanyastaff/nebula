//! In-memory `JobDispatchQueue` + `TriggerDedupInbox` over the shared
//! execution-store core.
//!
//! Both adapters wrap the same [`SharedState`] as the execution store and
//! control queue, so `claim_and_materialize_start` writes the dedup guard,
//! the execution row, and the Start job atomically in one critical section —
//! mirroring how `InMemoryControlQueue` shares `InMemoryExecutionStore`'s core.

use std::time::Duration;

use nebula_core::PluginKey;
use nebula_storage_port::dto::{
    DispatchKind, DispatchOutcome, JobDispatchMsg, NewExecution, TriggerDedupRow,
};
use nebula_storage_port::store::{
    ClaimGeneration, JobClaim, JobClaimToken, JobDispatchQueue, ReclaimOutcome, TriggerDedupInbox,
};
use nebula_storage_port::{Scope, StorageError};
use tokio::time::Instant;

use super::execution::{QueuedJob, SharedState, insert_created_row};

/// Format a raw 16-byte ULID as lowercase hex for `StorageError` ids. Uses
/// std formatting so the `inmem` module does not need the optional `hex` crate
/// that is only enabled by the `postgres`/`sqlite` features.
fn ulid_hex(id: &[u8; 16]) -> String {
    id.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Terminalise a claimed row, fenced on `(row id, Processing, generation)`.
///
/// Mirrors the SQL backends' single fenced `UPDATE` plus its zero-rows
/// disambiguation: an absent row is [`StorageError::NotFound`], and a present
/// row the token no longer owns is [`StorageError::FencedOut`] with **no state
/// change**. Returning `Ok` for a superseded token would let a worker whose
/// claim was reclaimed terminalise work the current owner is still doing.
fn acknowledge(
    state: &mut super::execution::State,
    claim: &JobClaimToken,
    terminal_status: &str,
    error: Option<&str>,
) -> Result<(), StorageError> {
    let id = claim.row_id();
    let Some(job) = state.jobs.get_mut(id) else {
        return Err(StorageError::NotFound {
            entity: "job_dispatch",
            id: ulid_hex(id),
        });
    };
    if job.status != "Processing" || job.claim_generation != claim.generation().get() {
        return Err(StorageError::FencedOut {
            entity: "job_dispatch",
            id: ulid_hex(id),
        });
    }
    terminal_status.clone_into(&mut job.status);
    if let Some(error) = error {
        job.error_message = Some(error.to_owned());
    }
    Ok(())
}

// ── JobDispatchQueue ─────────────────────────────────────────────────────────

/// In-memory job-dispatch queue handle.
///
/// Shares the execution store's core so `claim_and_materialize_start` (on the
/// dedup inbox side) operates in one critical section with the execution row
/// and job inserts.
#[derive(Debug, Clone)]
pub struct InMemoryJobDispatchQueue {
    inner: SharedState,
}

impl InMemoryJobDispatchQueue {
    /// Build a job-dispatch queue over an execution store's shared core.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
        }
    }
}

#[async_trait::async_trait]
impl JobDispatchQueue for InMemoryJobDispatchQueue {
    #[tracing::instrument(level = "debug", skip(self, msg), fields(id = ?msg.id, command = msg.command.as_str()))]
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        st.jobs.insert(
            msg.id,
            QueuedJob {
                msg: msg.clone(),
                status: "Pending".to_owned(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );
        tracing::debug!(target: "nebula_storage::inmem", "job_dispatch: enqueued");
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip(self, available_plugins), fields(batch_size))]
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        available_plugins: &[PluginKey],
    ) -> Result<Vec<JobClaim>, StorageError> {
        // Parity with SQLite + Postgres: an empty advertised set claims nothing.
        if available_plugins.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self.inner.lock();
        let now = Instant::now();

        // Stable order so a bounded batch is deterministic across calls.
        //
        // Superset predicate: the worker may claim a job only when its
        // available plugins cover every plugin in `required_plugins`.  The
        // check is inside the parking_lot Mutex so the predicate + status flip
        // are atomic (no TOCTOU window).  Empty `required_plugins` ⇒ `all()`
        // is vacuously true ⇒ claimable by any non-empty available set.
        let mut ids: Vec<[u8; 16]> = st
            .jobs
            .iter()
            .filter(|(_, q)| {
                q.status == "Pending"
                    && q.msg
                        .required_plugins
                        .iter()
                        .all(|rp| available_plugins.contains(rp))
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();

        let mut claimed = Vec::new();
        for id in ids.into_iter().take(batch_size as usize) {
            if let Some(q) = st.jobs.get_mut(&id) {
                // Mint the generation in the same critical section that flips
                // Pending -> Processing, so no other claimer can observe the
                // row as claimed under a generation that was not minted yet.
                // The SQL backends do this inside the claiming UPDATE.
                let Some(generation) = q.claim_generation.checked_add(1) else {
                    // Fail closed: a wrapped generation would let a superseded
                    // token match a future claim, which is exactly the fence
                    // this counter exists to provide.
                    return Err(StorageError::Internal(format!(
                        "job_dispatch claim generation overflowed for row {}",
                        ulid_hex(&id)
                    )));
                };
                q.claim_generation = generation;
                "Processing".clone_into(&mut q.status);
                q.processed_by = Some(*processor);
                q.processed_at = Some(now);
                q.msg.reclaim_count = q.reclaim_count;
                claimed.push(JobClaim {
                    msg: q.msg.clone(),
                    token: JobClaimToken::new(id, ClaimGeneration::new(generation)),
                });
            }
        }
        tracing::debug!(
            target: "nebula_storage::inmem",
            claimed = claimed.len(),
            "job_dispatch: claimed"
        );
        Ok(claimed)
    }

    async fn mark_dispatched(&self, claim: &JobClaimToken) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Dispatched", None)
    }

    async fn mark_failed(&self, claim: &JobClaimToken, error: &str) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        acknowledge(&mut st, claim, "Failed", Some(error))
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        let mut st = self.inner.lock();
        let now = Instant::now();
        let mut outcome = ReclaimOutcome::default();
        for q in st.jobs.values_mut() {
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
            if q.reclaim_count >= max_reclaim_count {
                "Failed".clone_into(&mut q.status);
                q.error_message = Some(format!(
                    "reclaim exhausted: presumed dead after {} reclaims",
                    q.reclaim_count
                ));
                outcome.exhausted += 1;
            } else {
                // Ownership is cleared, but `claim_generation` is deliberately
                // left alone: the next claim increments past it, so the token
                // this reclaim just invalidated can never match again.
                "Pending".clone_into(&mut q.status);
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
        // timestamps, so age-based pruning is a no-op (parity with
        // `InMemoryControlQueue`).
        Ok(0)
    }
}

// ── TriggerDedupInbox ────────────────────────────────────────────────────────

/// In-memory trigger-dedup inbox handle.
///
/// Shares the execution store's core with [`InMemoryJobDispatchQueue`] so
/// `claim_and_materialize_start` writes all three rows atomically under one
/// lock.
#[derive(Debug, Clone)]
pub struct InMemoryTriggerDedupInbox {
    inner: SharedState,
}

impl InMemoryTriggerDedupInbox {
    /// Build a trigger-dedup inbox over an execution store's shared core.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
        }
    }
}

#[async_trait::async_trait]
impl TriggerDedupInbox for InMemoryTriggerDedupInbox {
    #[tracing::instrument(level = "debug", skip(self, row, start, execution), fields(
        trigger_id = row.as_ref().map(|r| r.trigger_id.as_str()),
        event_id   = row.as_ref().map(|r| r.event_id.as_str()),
        job_id     = ?start.id,
        execution_id = start.execution_id.as_str(),
    ))]
    async fn claim_and_materialize_start(
        &self,
        row: Option<&TriggerDedupRow>,
        start: &JobDispatchMsg,
        execution: &NewExecution<'_>,
    ) -> Result<DispatchOutcome, StorageError> {
        let mut st = self.inner.lock();

        // All three writes are inside one critical section (the parking_lot
        // Mutex guard).
        //
        // Write order is important: `insert_created_row` MUST succeed before we
        // write to `st.dedup`.  The Mutex is not a database transaction — there
        // is no rollback.  If we inserted the dedup key first and then
        // `insert_created_row` failed (id collision), the dedup entry would stay
        // permanently, making the trigger permanently stuck as a "duplicate".
        //
        // Correct order:
        //  1. Duplicate check (read-only)
        //  2. insert_created_row — return Err immediately on failure; dedup untouched
        //  3. st.dedup.insert — only reachable on success
        //  4. st.jobs.insert

        // Step 1: check for an existing dedup winner and return early.
        let dedup_key = row.as_ref().map(|r| {
            (
                r.scope.workspace_id.clone(),
                r.scope.org_id.clone(),
                r.trigger_id.clone(),
                r.event_id.clone(),
            )
        });
        if let (Some(r), Some(key)) = (row, &dedup_key)
            && let Some(winner_id) = st.dedup.get(key)
        {
            let winner_id = winner_id.clone();
            tracing::debug!(
                target: "nebula_storage::inmem",
                trigger_id = %r.trigger_id,
                event_id   = %r.event_id,
                winner_execution_id = %winner_id,
                "trigger_dedup: duplicate — returning winner id"
            );
            return Ok(DispatchOutcome::new(winner_id, DispatchKind::Duplicate));
        }

        // Step 1b: reject a colliding job-dispatch id BEFORE materializing
        // anything. The SQL backends hit the job-dispatch primary key and roll
        // the whole transaction back, so the in-memory backend must fail closed
        // here too — otherwise the unconditional `st.jobs.insert` below would
        // silently overwrite the already-queued job and still report
        // `Dispatched`, diverging from SQL and losing the original job.
        if st.jobs.contains_key(&start.id) {
            return Err(StorageError::Duplicate {
                entity: "job_dispatch",
                detail: format!("job-dispatch id {:?} already queued", start.id),
            });
        }

        // Step 2: insert the execution row — fail-closed before touching dedup.
        // An id collision returns Err; neither dedup nor job maps are modified.
        insert_created_row(
            &mut st,
            &start.scope,
            &start.execution_id,
            execution.workflow_id,
            execution.initial_state,
        )?;

        // Step 3: claim the dedup slot (only reachable on success).
        if let Some(key) = dedup_key {
            st.dedup.insert(key, start.execution_id.clone());
        }

        st.jobs.insert(
            start.id,
            QueuedJob {
                msg: start.clone(),
                status: "Pending".to_owned(),
                processed_by: None,
                processed_at: None,
                reclaim_count: 0,
                error_message: None,
                claim_generation: 0,
            },
        );
        tracing::debug!(
            target: "nebula_storage::inmem",
            job_id = ?start.id,
            execution_id = %start.execution_id,
            "trigger_dedup: materialized (dedup guard + execution row + Start job)"
        );
        Ok(DispatchOutcome::new(
            start.execution_id.clone(),
            DispatchKind::Dispatched,
        ))
    }

    async fn exists(
        &self,
        scope: &Scope,
        trigger_id: &str,
        event_id: &str,
    ) -> Result<bool, StorageError> {
        let st = self.inner.lock();
        let key = (
            scope.workspace_id.clone(),
            scope.org_id.clone(),
            trigger_id.to_owned(),
            event_id.to_owned(),
        );
        Ok(st.dedup.contains_key(&key))
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // No-op stub — TTL sweep wired later without a trait break.
        Ok(0)
    }
}

#[cfg(test)]
mod job_ownership_tests {
    //! Acknowledgement must fail closed whenever the caller does not hold the
    //! row's current claim — an unknown row, or a token a reclaim superseded.
    //! A silent `Ok` would let a worker believe it dispatched a job another
    //! attempt now owns.
    //!
    //! A "wrong processor" case no longer exists by construction: authority is
    //! the storage-minted token, and a processor cannot fabricate one. The
    //! stronger ABA case (*same* processor, superseded generation) replaces it.

    use nebula_storage_port::dto::{ControlCommand, JobDispatchMsg};
    use nebula_storage_port::store::{ClaimGeneration, JobClaimToken, JobDispatchQueue};
    use nebula_storage_port::{Scope, StorageError as SE};

    use super::InMemoryJobDispatchQueue;
    use crate::inmem::InMemoryExecutionStore;

    fn make_queue() -> InMemoryJobDispatchQueue {
        let store = InMemoryExecutionStore::new();
        InMemoryJobDispatchQueue::new(&store)
    }

    fn sample_msg(id: [u8; 16]) -> JobDispatchMsg {
        JobDispatchMsg::new(
            id,
            "exec-1".to_owned(),
            ControlCommand::Start,
            Scope::new("ws-1", "org-1"),
            serde_json::Value::Null,
            None::<String>,
            String::new(),
            "plugin-a".parse().unwrap(),
            vec!["plugin-a".parse().unwrap()],
            None::<String>,
            0,
        )
    }

    #[tokio::test]
    async fn mark_dispatched_returns_not_found_for_unknown_job() {
        let queue = make_queue();
        let unknown = JobClaimToken::new([0xABu8; 16], ClaimGeneration::new(1));

        let result = queue.mark_dispatched(&unknown).await;
        assert!(
            matches!(
                result,
                Err(SE::NotFound {
                    entity: "job_dispatch",
                    ..
                })
            ),
            "mark_dispatched on an unknown job id must return NotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn mark_dispatched_is_fenced_out_after_the_claim_is_reclaimed() {
        let queue = make_queue();
        let worker_a: [u8; 16] = [1u8; 16];
        let job_id: [u8; 16] = [3u8; 16];

        queue.enqueue(&sample_msg(job_id)).await.unwrap();
        let claimed = queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        let stale = claimed[0].token;

        // The sweep hands the row back; the same worker claims it again. The
        // processor id is identical across both attempts, so only the
        // generation distinguishes them.
        tokio::time::pause();
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        let outcome = queue
            .reclaim_stuck(std::time::Duration::from_secs(1), 5)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 1, "the stuck row must be reclaimed");
        let reclaimed = queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();
        assert_eq!(reclaimed.len(), 1);
        assert!(
            reclaimed[0].token.generation() > stale.generation(),
            "a reclaimed row must mint a strictly greater generation"
        );

        let result = queue.mark_dispatched(&stale).await;
        assert!(
            matches!(
                result,
                Err(SE::FencedOut {
                    entity: "job_dispatch",
                    ..
                })
            ),
            "an acknowledgement from the superseded claim must be fenced out, got {result:?}"
        );

        // The fence must also be a no-op: the current claim still owns the row.
        assert!(
            queue.mark_dispatched(&reclaimed[0].token).await.is_ok(),
            "the current claim must still be able to acknowledge the row"
        );
    }

    #[tokio::test]
    async fn mark_failed_is_fenced_out_for_a_superseded_generation() {
        let queue = make_queue();
        let worker_a: [u8; 16] = [1u8; 16];
        let job_id: [u8; 16] = [4u8; 16];

        queue.enqueue(&sample_msg(job_id)).await.unwrap();
        let claimed = queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();
        let current = claimed[0].token;
        let superseded =
            JobClaimToken::new(job_id, ClaimGeneration::new(current.generation().get() - 1));

        let result = queue.mark_failed(&superseded, "some error").await;
        assert!(
            matches!(
                result,
                Err(SE::FencedOut {
                    entity: "job_dispatch",
                    ..
                })
            ),
            "mark_failed with a superseded generation must be fenced out, got {result:?}"
        );
    }

    #[tokio::test]
    async fn mark_dispatched_succeeds_for_the_current_claim() {
        let queue = make_queue();
        let worker_a: [u8; 16] = [1u8; 16];
        let job_id: [u8; 16] = [5u8; 16];

        queue.enqueue(&sample_msg(job_id)).await.unwrap();
        let claimed = queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();

        let result = queue.mark_dispatched(&claimed[0].token).await;
        assert!(
            result.is_ok(),
            "the claim's own token must acknowledge the row, got {result:?}"
        );
    }
}
