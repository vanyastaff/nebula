//! In-memory `JobDispatchQueue` + `TriggerDedupInbox` over a shared core.
//!
//! Both adapters wrap the same [`SharedDispatchCore`] so
//! `claim_and_enqueue_start` performs the dedup-insert ∧ job-insert in one
//! critical section — mirroring how `InMemoryControlQueue` shares
//! `InMemoryExecutionStore`'s core.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

use nebula_storage_port::dto::{CapabilityTag, DispatchOutcome, JobDispatchMsg, TriggerDedupRow};
use nebula_storage_port::store::{JobDispatchQueue, ReclaimOutcome, TriggerDedupInbox};
use nebula_storage_port::{Scope, StorageError};

// ── shared core ─────────────────────────────────────────────────────────────

/// One queued job row plus its processing bookkeeping.
#[derive(Debug, Clone)]
struct QueuedJob {
    msg: JobDispatchMsg,
    status: String,
    processed_by: Option<[u8; 16]>,
    processed_at: Option<Instant>,
    reclaim_count: u32,
    error_message: Option<String>,
}

#[derive(Debug, Default)]
struct Core {
    /// Job-dispatch queue rows keyed by the message's 16-byte id.
    jobs: HashMap<[u8; 16], QueuedJob>,
    /// Dedup guard set keyed by `(workspace_id, org_id, trigger_id, event_id)`.
    ///
    /// The scope columns are part of the key — identical `(trigger_id, event_id)`
    /// values under different tenants are independent entries, mirroring the
    /// `PRIMARY KEY (workspace_id, org_id, trigger_id, event_id)` constraint in
    /// both the SQLite and Postgres schemas.
    dedup: HashSet<(String, String, String, String)>,
}

/// Opaque shared dispatch core handle.
///
/// Both [`InMemoryJobDispatchQueue`] and [`InMemoryTriggerDedupInbox`] wrap
/// this handle so `claim_and_enqueue_start` operates in one critical section.
/// Construct with [`new_shared_core`] and pass the clone to both adapters.
#[derive(Debug, Clone)]
pub struct SharedDispatchCore(Arc<Mutex<Core>>);

/// Construct a fresh shared dispatch core.  Pass the same handle to
/// [`InMemoryJobDispatchQueue::from_core`] and
/// [`InMemoryTriggerDedupInbox::from_core`].
#[must_use]
pub fn new_shared_core() -> SharedDispatchCore {
    SharedDispatchCore(Arc::new(Mutex::new(Core::default())))
}

// ── JobDispatchQueue ─────────────────────────────────────────────────────────

/// In-memory job-dispatch queue handle.
#[derive(Debug, Clone)]
pub struct InMemoryJobDispatchQueue {
    core: SharedDispatchCore,
}

impl InMemoryJobDispatchQueue {
    /// Build from an existing shared core (share with
    /// [`InMemoryTriggerDedupInbox`] so `claim_and_enqueue_start` is atomic).
    #[must_use]
    pub fn from_core(core: SharedDispatchCore) -> Self {
        Self { core }
    }

    fn lock(&self) -> parking_lot::MutexGuard<'_, Core> {
        self.core.0.lock()
    }
}

#[async_trait::async_trait]
impl JobDispatchQueue for InMemoryJobDispatchQueue {
    #[tracing::instrument(level = "debug", skip(self, msg), fields(id = ?msg.id, command = msg.command.as_str()))]
    async fn enqueue(&self, msg: &JobDispatchMsg) -> Result<(), StorageError> {
        let mut st = self.lock();
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

    #[tracing::instrument(level = "debug", skip(self, advertised_tags), fields(batch_size))]
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        advertised_tags: &[CapabilityTag],
    ) -> Result<Vec<JobDispatchMsg>, StorageError> {
        let mut st = self.lock();
        let now = Instant::now();

        // Stable order so a bounded batch is deterministic across calls.
        let mut ids: Vec<[u8; 16]> = st
            .jobs
            .iter()
            .filter(|(_, q)| {
                q.status == "Pending"
                    && advertised_tags
                        .iter()
                        .any(|t| t.as_str() == q.msg.required_plugin_key)
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
        let mut st = self.lock();
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
        let mut st = self.lock();
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
        let mut st = self.lock();
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

/// In-memory trigger-dedup inbox handle.  Shares `core` with
/// [`InMemoryJobDispatchQueue`] so `claim_and_enqueue_start` is one
/// critical section.
#[derive(Debug, Clone)]
pub struct InMemoryTriggerDedupInbox {
    core: SharedDispatchCore,
}

impl InMemoryTriggerDedupInbox {
    /// Build from an existing shared core (share with
    /// [`InMemoryJobDispatchQueue`] so `claim_and_enqueue_start` is atomic).
    #[must_use]
    pub fn from_core(core: SharedDispatchCore) -> Self {
        Self { core }
    }

    fn lock(&self) -> parking_lot::MutexGuard<'_, Core> {
        self.core.0.lock()
    }
}

#[async_trait::async_trait]
impl TriggerDedupInbox for InMemoryTriggerDedupInbox {
    #[tracing::instrument(level = "debug", skip(self, row, start), fields(
        trigger_id = row.as_ref().map(|r| r.trigger_id.as_str()),
        event_id   = row.as_ref().map(|r| r.event_id.as_str()),
        job_id     = ?start.id,
    ))]
    async fn claim_and_enqueue_start(
        &self,
        row: Option<&TriggerDedupRow>,
        start: &JobDispatchMsg,
    ) -> Result<DispatchOutcome, StorageError> {
        let mut st = self.lock();
        // One critical section — both writes are inside the same lock.
        if let Some(r) = row {
            let key = (
                r.scope.workspace_id.clone(),
                r.scope.org_id.clone(),
                r.trigger_id.clone(),
                r.event_id.clone(),
            );
            if st.dedup.contains(&key) {
                tracing::debug!(
                    target: "nebula_storage::inmem",
                    trigger_id = %r.trigger_id,
                    event_id   = %r.event_id,
                    "trigger_dedup: duplicate"
                );
                return Ok(DispatchOutcome::Duplicate);
            }
            st.dedup.insert(key);
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
            "trigger_dedup: dispatched"
        );
        Ok(DispatchOutcome::Dispatched)
    }

    async fn exists(
        &self,
        scope: &Scope,
        trigger_id: &str,
        event_id: &str,
    ) -> Result<bool, StorageError> {
        let st = self.lock();
        let key = (
            scope.workspace_id.clone(),
            scope.org_id.clone(),
            trigger_id.to_owned(),
            event_id.to_owned(),
        );
        let found = st.dedup.contains(&key);
        Ok(found)
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // No-op stub — TTL sweep wired later without a trait break.
        Ok(0)
    }
}
