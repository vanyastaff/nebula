//! Append-only execution journal.

use async_trait::async_trait;

use crate::error::StorageError;

/// Journal entry — the on-disk shape.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    /// 16-byte BYTEA (ULID), monotonic within an execution.
    pub id: Vec<u8>,
    /// Parent execution.
    pub execution_id: Vec<u8>,
    /// Per-execution monotonic sequence counter.
    pub sequence: i64,
    /// Event type discriminator (e.g. `'ExecutionStarted'`, `'NodeFinished'`).
    pub event_type: String,
    /// Optional node attempt that triggered this event.
    pub node_attempt_id: Option<Vec<u8>>,
    /// Event payload.
    pub payload: serde_json::Value,
    /// When the event was emitted (UTC).
    pub emitted_at: chrono::DateTime<chrono::Utc>,
}

/// Append-only storage for execution events.
///
/// Spec 16 layer 4. This is the replayable history operators inspect
/// to answer *what happened*. No UPDATE or DELETE in runtime code —
/// retention is by cascade on `executions`.
#[async_trait]
pub trait JournalRepo: Send + Sync {
    /// Append an entry. Auto-assigns `sequence` as the next value for
    /// the execution.
    async fn append(&self, entry: &JournalEntry) -> Result<(), StorageError>;

    /// Batch-append multiple entries atomically.
    async fn append_batch(&self, entries: &[JournalEntry]) -> Result<(), StorageError>;

    /// Read the full journal for an execution, ordered by `sequence`.
    async fn list_for_execution(
        &self,
        execution_id: &[u8],
    ) -> Result<Vec<JournalEntry>, StorageError>;

    /// Read entries after a given sequence (for streaming/catch-up).
    async fn list_after(
        &self,
        execution_id: &[u8],
        after_sequence: i64,
        limit: u32,
    ) -> Result<Vec<JournalEntry>, StorageError>;
}
