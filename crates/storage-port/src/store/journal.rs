//! Execution-journal read trait.
use crate::dto::JournalEntry;
use crate::error::StorageError;
use crate::scope::Scope;

/// Read-only view over the append-only execution journal. Appends happen
/// through [`crate::TransitionBatch`] or a backend-authored, deduplicated control
/// refusal inside its verified aggregate transaction. Refused actors never
/// receive independent journal mutation authority.
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
