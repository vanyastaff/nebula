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
        let mut st = self.inner.lock();
        if let Some(q) = st.jobs.get_mut(id)
            && q.status == "Processing"
            && q.processed_by.as_ref() == Some(processor)
        {
            q.status = "Dispatched".to_owned();
        }
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        if let Some(q) = st.jobs.get_mut(id)
            && q.status == "Processing"
            && q.processed_by.as_ref() == Some(processor)
        {
            q.status = "Failed".to_owned();
            q.error_message = Some(error.to_owned());
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
