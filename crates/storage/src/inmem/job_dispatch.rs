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
use nebula_storage_port::store::{JobDispatchQueue, ReclaimOutcome, TriggerDedupInbox};
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
    ) -> Result<Vec<JobDispatchMsg>, StorageError> {
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
                q.status = "Processing".to_owned();
                q.processed_by = Some(*processor);
                q.processed_at = Some(now);
                q.msg.reclaim_count = q.reclaim_count;
                claimed.push(q.msg.clone());
            }
        }
        tracing::debug!(
            target: "nebula_storage::inmem",
            claimed = claimed.len(),
            "job_dispatch: claimed"
        );
        Ok(claimed)
    }

    async fn mark_dispatched(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        // Fail-closed: if this worker no longer owns the job (reclaimed by
        // another or the job id is absent), return NotFound — mirrors the SQL
        // backends' `rows_affected == 0` check. A silent Ok would let a worker
        // that lost ownership believe it successfully dispatched.
        let mut st = self.inner.lock();
        if let Some(q) = st.jobs.get_mut(id)
            && q.status == "Processing"
            && q.processed_by.as_ref() == Some(processor)
        {
            q.status = "Dispatched".to_owned();
            return Ok(());
        }
        Err(StorageError::NotFound {
            entity: "job_dispatch",
            id: ulid_hex(id),
        })
    }

    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError> {
        // Fail-closed: same NotFound semantics as mark_dispatched.
        let mut st = self.inner.lock();
        if let Some(q) = st.jobs.get_mut(id)
            && q.status == "Processing"
            && q.processed_by.as_ref() == Some(processor)
        {
            q.status = "Failed".to_owned();
            q.error_message = Some(error.to_owned());
            return Ok(());
        }
        Err(StorageError::NotFound {
            entity: "job_dispatch",
            id: ulid_hex(id),
        })
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
                q.status = "Failed".to_owned();
                q.error_message = Some(format!(
                    "reclaim exhausted: presumed dead after {} reclaims",
                    q.reclaim_count
                ));
                outcome.exhausted += 1;
            } else {
                q.status = "Pending".to_owned();
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
    //! FIX 3 regression: mark_dispatched / mark_failed must return Err (not Ok)
    //! when the caller does not own the job (wrong processor id, reclaimed row,
    //! or unknown job id). A silent Ok would let a worker believe it dispatched
    //! a job it no longer holds.

    use nebula_storage_port::dto::{ControlCommand, JobDispatchMsg};
    use nebula_storage_port::store::JobDispatchQueue;
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
        let worker_a: [u8; 16] = [1u8; 16];
        let unknown_id: [u8; 16] = [0xABu8; 16];

        let result = queue.mark_dispatched(&unknown_id, &worker_a).await;
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
    async fn mark_dispatched_returns_not_found_when_owned_by_different_processor() {
        let queue = make_queue();
        let worker_a: [u8; 16] = [1u8; 16];
        let worker_b: [u8; 16] = [2u8; 16];
        let job_id: [u8; 16] = [3u8; 16];

        let msg = sample_msg(job_id);
        queue.enqueue(&msg).await.unwrap();

        // Worker A claims the job.
        let claimed = queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);

        // Worker B tries to mark it dispatched — must fail.
        let result = queue.mark_dispatched(&job_id, &worker_b).await;
        assert!(
            matches!(
                result,
                Err(SE::NotFound {
                    entity: "job_dispatch",
                    ..
                })
            ),
            "mark_dispatched by a non-owner processor must return NotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn mark_failed_returns_not_found_when_owned_by_different_processor() {
        let queue = make_queue();
        let worker_a: [u8; 16] = [1u8; 16];
        let worker_b: [u8; 16] = [2u8; 16];
        let job_id: [u8; 16] = [4u8; 16];

        let msg = sample_msg(job_id);
        queue.enqueue(&msg).await.unwrap();
        queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();

        // Worker B tries to mark it failed — must fail.
        let result = queue.mark_failed(&job_id, &worker_b, "some error").await;
        assert!(
            matches!(
                result,
                Err(SE::NotFound {
                    entity: "job_dispatch",
                    ..
                })
            ),
            "mark_failed by a non-owner processor must return NotFound, got {result:?}"
        );
    }

    #[tokio::test]
    async fn mark_dispatched_succeeds_for_owning_processor() {
        let queue = make_queue();
        let worker_a: [u8; 16] = [1u8; 16];
        let job_id: [u8; 16] = [5u8; 16];

        let msg = sample_msg(job_id);
        queue.enqueue(&msg).await.unwrap();
        queue
            .claim_pending(&worker_a, 1, &["plugin-a".parse().unwrap()])
            .await
            .unwrap();

        let result = queue.mark_dispatched(&job_id, &worker_a).await;
        assert!(
            result.is_ok(),
            "mark_dispatched by the owning processor must succeed, got {result:?}"
        );
    }
}
