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
    /// When processing finished.
    pub processed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Error message if processing failed.
    pub error_message: Option<String>,
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
        _processor: &[u8],
        batch_size: u32,
    ) -> Result<Vec<ControlQueueEntry>, StorageError> {
        let mut entries = self.entries.lock().await;
        let pending: Vec<ControlQueueEntry> = entries
            .iter()
            .filter(|e| e.status == "Pending")
            .take(batch_size as usize)
            .cloned()
            .collect();
        // Transition matched rows to Processing.
        for e in &pending {
            if let Some(row) = entries.iter_mut().find(|r| r.id == e.id) {
                row.status = "Processing".to_string();
            }
        }
        Ok(pending)
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

    async fn cleanup(&self, _retention: std::time::Duration) -> Result<u64, StorageError> {
        // In-memory entries have no real timestamps for age-based pruning; no-op.
        Ok(0)
    }
}
