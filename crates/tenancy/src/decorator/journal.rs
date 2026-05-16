//! Scope-enforcing [`ExecutionJournalReader`] decorator.

use std::sync::Arc;

use nebula_storage_port::dto::JournalEntry;
use nebula_storage_port::store::ExecutionJournalReader;
use nebula_storage_port::{Scope, StorageError};

/// Wraps an [`ExecutionJournalReader`] and forces every read into the
/// bound [`Scope`].
///
/// A journal read for an execution that exists only in another tenant
/// resolves under the bound scope and comes back empty — the journal
/// never leaks cross-tenant history (§6.1 confused-deputy).
#[derive(Clone)]
pub struct ScopedExecutionJournalReader {
    inner: Arc<dyn ExecutionJournalReader>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedExecutionJournalReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedExecutionJournalReader")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedExecutionJournalReader {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn ExecutionJournalReader>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ExecutionJournalReader for ScopedExecutionJournalReader {
    async fn get_journal(
        &self,
        _scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        self.inner.get_journal(&self.bound, execution_id).await
    }

    async fn list_after(
        &self,
        _scope: &Scope,
        execution_id: &str,
        after: u64,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        self.inner
            .list_after(&self.bound, execution_id, after)
            .await
    }
}
