//! In-memory `ControlQueue` over the shared execution-store core.
//!
//! Built from [`super::InMemoryExecutionStore::shared`] so a `commit`'s
//! outbox rows are immediately claimable. Ids are typed 16-byte ULIDs
//! (`[u8; 16]`) — there is no UTF-8-of-ULID encoding. `enqueue` carries
//! the tenant `Scope`; `mark_completed`/`mark_failed` are fenced by the
//! claiming processor so a reclaimed-then-stale runner cannot overwrite a
//! newer claim.

use std::time::{Duration, Instant};

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::ControlMsg;
use nebula_storage_port::store::{ControlQueue, ReclaimOutcome};

use super::execution::{QueuedMsg, SharedState};

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
    ) -> Result<Vec<ControlMsg>, StorageError> {
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
                q.status = "Processing".to_string();
                q.processed_by = Some(*processor);
                q.processed_at = Some(now);
                claimed.push(q.msg.clone());
            }
        }
        Ok(claimed)
    }

    async fn mark_completed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        if let Some(q) = st.queue.get_mut(id)
            && q.status == "Processing"
            && q.processed_by.as_ref() == Some(processor)
        {
            q.status = "Completed".to_string();
        }
        // A processor mismatch is an idempotent no-op under the
        // at-least-once contract (a stale runner whose row was reclaimed
        // must not flip a newer claim).
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError> {
        let mut st = self.inner.lock();
        if let Some(q) = st.queue.get_mut(id)
            && q.status == "Processing"
            && q.processed_by.as_ref() == Some(processor)
        {
            q.status = "Failed".to_string();
            q.error_message = Some(error.to_string());
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
            if q.reclaim_count >= max_reclaim_count {
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
