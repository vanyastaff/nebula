//! Execution control queue (durable outbox pattern).
//!
//! Spec 16 §12.2. Every `Cancel`/`Terminate`/`Resume`/`Restart`
//! signal is written here atomically with the state transition that
//! caused it. A dispatcher drains pending rows and forwards them
//! to the engine.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::StorageError;

/// Control commands delivered through the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
    /// First-time dispatch of a newly-created execution.
    ///
    /// Enqueued by the API `start_execution` / `execute_workflow` handlers
    /// once the `ExecutionState::Created` row has been persisted (canon §12.2,
    /// §13 step 3, #332). The engine-side consumer picks this up and drives
    /// the execution through its initial transition to `Running` — closing
    /// the §4.5 public-surface gap where the API advertised workflow
    /// dispatch but never reached the engine.
    Start,
    /// Cooperative cancel (graceful shutdown of running work).
    Cancel,
    /// Forced termination (escalation after grace period).
    Terminate,
    /// Resume a suspended execution.
    Resume,
    /// Restart an execution from the beginning.
    Restart,
}

impl ControlCommand {
    /// Serialize to the text value stored in `execution_control_queue.command`.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Cancel => "Cancel",
            Self::Terminate => "Terminate",
            Self::Resume => "Resume",
            Self::Restart => "Restart",
        }
    }
}

/// Queued control command record.
///
/// # Invariant: ID Encoding
///
/// All byte-slice ID fields (`execution_id`) are currently stored as **UTF-8 bytes** of the
/// identifier's ULID string (e.g., `ExecutionId::to_string().into_bytes()`), NOT raw 16-byte
/// ULID values. Consumers must decode via `str::from_utf8` and parse into the corresponding ID
/// type. When a Postgres implementation lands, producers and consumers must be updated atomically
/// to preserve this encoding (as `TEXT` column or `BYTEA` of the ASCII string), or migrated
/// together to raw 16-byte ULIDs.
#[derive(Debug, Clone)]
pub struct ControlQueueEntry {
    /// 16-byte BYTEA (ULID) primary key.
    pub id: Vec<u8>,
    /// Target execution. Encoded as UTF-8 bytes of the ULID string.
    pub execution_id: Vec<u8>,
    /// The command to deliver.
    pub command: ControlCommand,
    /// Principal who issued the command (user or service account).
    pub issued_by: Option<Vec<u8>>,
    /// When the command was enqueued.
    pub issued_at: chrono::DateTime<chrono::Utc>,
    /// Current processing state.
    pub status: String,
    /// Node/instance that processed the command.
    pub processed_by: Option<Vec<u8>>,
    /// When this row was claimed for processing (stamped by `claim_pending`).
    ///
    /// Used by [`ControlQueueRepo::reclaim_stuck`] as the staleness signal
    /// for crashed-runner recovery — rows whose `processed_at` is older
    /// than the `reclaim_after` window are redelivered. Cleared on a
    /// successful reclaim so the next `claim_pending` resets the clock.
    /// See ADR-0017 / ADR-0008 B1.
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Error message if processing failed.
    pub error_message: Option<String>,
    /// Number of times this row has been reclaimed back to `Pending` after a
    /// crashed runner left it in `Processing` (ADR-0017, ADR-0008 B1). Bounded
    /// by `max_reclaim_count` on the consumer; rows past the budget move to
    /// `Failed` with a `"reclaim exhausted:"` error.
    pub reclaim_count: u32,
}

/// Summary of a single `reclaim_stuck` sweep (ADR-0017).
///
/// `reclaimed` counts rows moved `Processing → Pending` for a fresh dispatch
/// attempt; `exhausted` counts rows moved `Processing → Failed` because
/// their `reclaim_count` reached or exceeded `max_reclaim_count`. Both are
/// per-sweep counters — callers aggregate across ticks for observability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReclaimOutcome {
    /// Rows transitioned back to `Pending` for redelivery.
    pub reclaimed: u64,
    /// Rows transitioned to `Failed` because `reclaim_count` >= `max_reclaim_count`.
    pub exhausted: u64,
}

/// Durable control-signal outbox.
#[async_trait]
pub trait ControlQueueRepo: Send + Sync {
    /// Enqueue a new control command.
    async fn enqueue(&self, entry: &ControlQueueEntry) -> Result<(), StorageError>;

    /// Atomically claim up to `batch_size` pending commands.
    ///
    /// Rows transition `Pending → Processing` and record `processed_by`.
    /// Implementations must use `FOR UPDATE SKIP LOCKED` on Postgres.
    async fn claim_pending(
        &self,
        processor: &[u8],
        batch_size: u32,
    ) -> Result<Vec<ControlQueueEntry>, StorageError>;

    /// Mark a claimed command as successfully processed.
    async fn mark_completed(&self, id: &[u8]) -> Result<(), StorageError>;

    /// Mark a claimed command as failed (records `error_message`).
    async fn mark_failed(&self, id: &[u8], error: &str) -> Result<(), StorageError>;

    /// Reclaim rows stuck in `Processing` whose owning runner is presumed
    /// dead (ADR-0017, ADR-0008 B1).
    ///
    /// Finds rows where `status = 'Processing'` and
    /// `processed_at < now - reclaim_after`. For each such row:
    ///
    /// - If `reclaim_count < max_reclaim_count`: transition back to `Pending`, bump
    ///   `reclaim_count`, clear `processed_at` + `processed_by`. Row becomes claimable by the next
    ///   `claim_pending`.
    /// - Otherwise: transition to `Failed` with error message `"reclaim exhausted: processor
    ///   <processor_id> presumed dead after <N> reclaims"`.
    ///
    /// Safe under concurrent runners — the CAS on the status transition
    /// fences duplicates. Returns a [`ReclaimOutcome`] summarising the
    /// sweep.
    async fn reclaim_stuck(
        &self,
        reclaim_after: std::time::Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError>;

    /// Delete rows older than `retention`. Returns count deleted.
    async fn cleanup(&self, retention: std::time::Duration) -> Result<u64, StorageError>;
}

/// In-memory control queue repository for tests and development servers.
///
/// All operations are backed by a `Vec` guarded by a `Mutex`. Not suitable
/// for production — use the Postgres implementation instead.
#[derive(Debug, Default, Clone)]
pub struct InMemoryControlQueueRepo {
    entries: Arc<Mutex<Vec<ControlQueueEntry>>>,
}

impl InMemoryControlQueueRepo {
    /// Create an empty in-memory control queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of all enqueued entries (for test assertions).
    pub async fn snapshot(&self) -> Vec<ControlQueueEntry> {
        self.entries.lock().await.clone()
    }
}

#[async_trait]
impl ControlQueueRepo for InMemoryControlQueueRepo {
    async fn enqueue(&self, entry: &ControlQueueEntry) -> Result<(), StorageError> {
        self.entries.lock().await.push(entry.clone());
        Ok(())
    }

    async fn claim_pending(
        &self,
        processor: &[u8],
        batch_size: u32,
    ) -> Result<Vec<ControlQueueEntry>, StorageError> {
        let mut entries = self.entries.lock().await;
        let now = chrono::Utc::now();
        let pending: Vec<ControlQueueEntry> = entries
            .iter()
            .filter(|e| e.status == "Pending")
            .take(batch_size as usize)
            .cloned()
            .collect();
        for e in &pending {
            if let Some(row) = entries.iter_mut().find(|r| r.id == e.id) {
                row.status = "Processing".to_string();
                row.processed_at = Some(now);
                row.processed_by = Some(processor.to_vec());
            }
        }
        // Return the up-to-date snapshot (stamped), not the pre-update clone.
        Ok(pending
            .into_iter()
            .filter_map(|e| entries.iter().find(|r| r.id == e.id).cloned())
            .collect())
    }

    async fn mark_completed(&self, id: &[u8]) -> Result<(), StorageError> {
        let mut entries = self.entries.lock().await;
        if let Some(row) = entries.iter_mut().find(|e| e.id == id) {
            row.status = "Completed".to_string();
        }
        Ok(())
    }

    async fn mark_failed(&self, id: &[u8], error: &str) -> Result<(), StorageError> {
        let mut entries = self.entries.lock().await;
        if let Some(row) = entries.iter_mut().find(|e| e.id == id) {
            row.status = "Failed".to_string();
            row.error_message = Some(error.to_string());
        }
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: std::time::Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        let mut entries = self.entries.lock().await;
        let cutoff = chrono::Utc::now()
            - chrono::Duration::from_std(reclaim_after).unwrap_or(chrono::Duration::zero());
        let mut outcome = ReclaimOutcome::default();

        for row in entries.iter_mut() {
            if row.status != "Processing" {
                continue;
            }
            let Some(ts) = row.processed_at else {
                continue;
            };
            if ts >= cutoff {
                continue;
            }

            if row.reclaim_count >= max_reclaim_count {
                let processor = row
                    .processed_by
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .unwrap_or_else(|| "<unknown>".to_string());
                row.status = "Failed".to_string();
                row.error_message = Some(format!(
                    "reclaim exhausted: processor {processor} presumed dead after {} reclaims",
                    row.reclaim_count
                ));
                outcome.exhausted += 1;
            } else {
                row.status = "Pending".to_string();
                row.reclaim_count = row.reclaim_count.saturating_add(1);
                row.processed_at = None;
                row.processed_by = None;
                outcome.reclaimed += 1;
            }
        }

        Ok(outcome)
    }

    async fn cleanup(&self, _retention: std::time::Duration) -> Result<u64, StorageError> {
        // In-memory entries have no real timestamps for age-based pruning; no-op.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claim_pending_stamps_processed_at_and_processed_by() {
        let repo = InMemoryControlQueueRepo::new();
        let entry = ControlQueueEntry {
            id: vec![1u8; 16],
            execution_id: b"01JXYZ00000000000000000000".to_vec(),
            command: ControlCommand::Cancel,
            issued_by: None,
            issued_at: chrono::Utc::now(),
            status: "Pending".to_string(),
            processed_by: None,
            processed_at: None,
            error_message: None,
            reclaim_count: 0,
        };
        repo.enqueue(&entry).await.unwrap();

        let before = chrono::Utc::now();
        let claimed = repo.claim_pending(b"runner-a", 16).await.unwrap();
        let after = chrono::Utc::now();
        assert_eq!(claimed.len(), 1);

        let snap = repo.snapshot().await;
        let row = snap.iter().find(|r| r.id == vec![1u8; 16]).unwrap();
        assert_eq!(row.status, "Processing");
        assert_eq!(row.processed_by.as_deref(), Some(b"runner-a".as_slice()));
        let ts = row.processed_at.expect("processed_at stamped");
        assert!(
            ts >= before && ts <= after,
            "processed_at inside the claim window"
        );
    }

    fn enqueued(
        id: u8,
        status: &str,
        processed_at: Option<chrono::DateTime<chrono::Utc>>,
        reclaim_count: u32,
    ) -> ControlQueueEntry {
        ControlQueueEntry {
            id: vec![id; 16],
            execution_id: b"01JXYZ00000000000000000000".to_vec(),
            command: ControlCommand::Cancel,
            issued_by: None,
            issued_at: chrono::Utc::now(),
            status: status.to_string(),
            processed_by: Some(b"dead-runner".to_vec()),
            processed_at,
            error_message: None,
            reclaim_count,
        }
    }

    #[tokio::test]
    async fn reclaim_stuck_moves_expired_processing_to_pending() {
        let repo = InMemoryControlQueueRepo::new();
        let stale = chrono::Utc::now() - chrono::Duration::seconds(600);
        repo.enqueue(&enqueued(1, "Processing", Some(stale), 0))
            .await
            .unwrap();

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 1);
        assert_eq!(outcome.exhausted, 0);

        let snap = repo.snapshot().await;
        let row = snap.iter().find(|r| r.id == vec![1u8; 16]).unwrap();
        assert_eq!(row.status, "Pending", "reclaimed back to Pending");
        assert_eq!(row.reclaim_count, 1, "reclaim_count bumped");
        assert!(
            row.processed_by.is_none(),
            "processed_by cleared on reclaim"
        );
        assert!(
            row.processed_at.is_none(),
            "processed_at cleared on reclaim"
        );
    }

    #[tokio::test]
    async fn reclaim_stuck_leaves_fresh_processing_alone() {
        let repo = InMemoryControlQueueRepo::new();
        let fresh = chrono::Utc::now() - chrono::Duration::seconds(10);
        repo.enqueue(&enqueued(2, "Processing", Some(fresh), 0))
            .await
            .unwrap();

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 0);
        assert_eq!(outcome.exhausted, 0);

        let snap = repo.snapshot().await;
        let row = snap.iter().find(|r| r.id == vec![2u8; 16]).unwrap();
        assert_eq!(row.status, "Processing", "fresh row untouched");
        assert_eq!(row.reclaim_count, 0);
    }

    #[tokio::test]
    async fn reclaim_stuck_leaves_non_processing_rows_alone() {
        let repo = InMemoryControlQueueRepo::new();
        let stale = chrono::Utc::now() - chrono::Duration::seconds(600);
        repo.enqueue(&enqueued(3, "Completed", Some(stale), 0))
            .await
            .unwrap();
        repo.enqueue(&enqueued(4, "Failed", Some(stale), 0))
            .await
            .unwrap();
        repo.enqueue(&enqueued(5, "Pending", None, 0))
            .await
            .unwrap();

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 0);
        assert_eq!(outcome.exhausted, 0);
    }

    #[tokio::test]
    async fn reclaim_stuck_exhausts_after_max_count() {
        let repo = InMemoryControlQueueRepo::new();
        let stale = chrono::Utc::now() - chrono::Duration::seconds(600);
        repo.enqueue(&enqueued(6, "Processing", Some(stale), 3))
            .await
            .unwrap();

        let outcome = repo
            .reclaim_stuck(std::time::Duration::from_secs(150), 3)
            .await
            .unwrap();
        assert_eq!(outcome.reclaimed, 0, "not requeued — past budget");
        assert_eq!(outcome.exhausted, 1, "moved to Failed as exhausted");

        let snap = repo.snapshot().await;
        let row = snap.iter().find(|r| r.id == vec![6u8; 16]).unwrap();
        assert_eq!(row.status, "Failed");
        let msg = row.error_message.as_deref().expect("error_message set");
        assert!(
            msg.starts_with("reclaim exhausted: "),
            "canonical prefix, got: {msg}"
        );
        assert!(
            msg.contains("dead-runner"),
            "includes processor_id, got: {msg}"
        );
    }
}
