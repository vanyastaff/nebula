//! In-memory `ExecutionJournalReader` over the shared execution-store
//! core.
//!
//! Built from [`super::InMemoryExecutionStore::shared`] so the journal
//! entries a `commit` appended are immediately readable. Scope is enforced
//! exactly as the SQL backends enforce it: a cross-tenant read returns an
//! empty journal (never another tenant's entries).

use nebula_storage_port::dto::JournalEntry;
use nebula_storage_port::store::ExecutionJournalReader;
use nebula_storage_port::{Scope, StorageError};

use super::execution::SharedState;

/// In-memory journal-read handle. Shares the execution store's core.
#[derive(Debug, Clone)]
pub struct InMemoryJournalReader {
    inner: SharedState,
}

impl InMemoryJournalReader {
    /// Build a journal reader over an execution store's shared core.
    #[must_use]
    pub fn new(store: &super::InMemoryExecutionStore) -> Self {
        Self {
            inner: store.shared(),
        }
    }
}

#[async_trait::async_trait]
impl ExecutionJournalReader for InMemoryJournalReader {
    async fn get_journal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        let st = self.inner.lock();
        match st.rows.get(execution_id) {
            Some(row) if &row.scope == scope => Ok(row
                .journal
                .iter()
                .map(|(seq, payload)| JournalEntry {
                    seq: Some(*seq),
                    payload: payload.clone(),
                })
                .collect()),
            // Absent or cross-tenant: an empty journal, never a leak.
            _ => Ok(Vec::new()),
        }
    }

    async fn list_after(
        &self,
        scope: &Scope,
        execution_id: &str,
        after: u64,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        let st = self.inner.lock();
        match st.rows.get(execution_id) {
            Some(row) if &row.scope == scope => Ok(row
                .journal
                .iter()
                .filter(|(seq, _)| *seq > after)
                .map(|(seq, payload)| JournalEntry {
                    seq: Some(*seq),
                    payload: payload.clone(),
                })
                .collect()),
            _ => Ok(Vec::new()),
        }
    }
}
