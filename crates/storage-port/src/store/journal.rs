//! Execution-journal read trait.
use crate::dto::JournalEntry;
use crate::error::StorageError;
use crate::scope::Scope;

/// Read-only view over the append-only execution journal. Appends happen
/// only through [`crate::TransitionBatch`] so the journal can never diverge
/// from the state it describes.
#[async_trait::async_trait]
pub trait ExecutionJournalReader: Send + Sync + std::fmt::Debug {
    /// Full journal for an execution, oldest first.
    async fn get_journal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<JournalEntry>, StorageError>;

    /// Journal entries with `seq` strictly greater than `after`.
    async fn list_after(
        &self,
        scope: &Scope,
        execution_id: &str,
        after: u64,
    ) -> Result<Vec<JournalEntry>, StorageError>;
}
