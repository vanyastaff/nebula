//! Execution control queue (durable outbox pattern).
//!
//! Spec 16 §12.2. Every `Cancel`/`Terminate`/`Resume`/`Restart`
//! signal is written here atomically with the state transition that
//! caused it. A dispatcher drains pending rows and forwards them
//! to the engine.

use async_trait::async_trait;

use crate::error::StorageError;

/// Control commands delivered through the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand {
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
            Self::Cancel => "Cancel",
            Self::Terminate => "Terminate",
            Self::Resume => "Resume",
            Self::Restart => "Restart",
        }
    }
}

/// Queued control command record.
#[derive(Debug, Clone)]
pub struct ControlQueueEntry {
    /// 16-byte BYTEA (ULID) primary key.
    pub id: Vec<u8>,
    /// Target execution.
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
