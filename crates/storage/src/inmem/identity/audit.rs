//! Audit log: append-only; reads are newest-first.

use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::AuditLogRow;
use nebula_storage_port::store::AuditStore;
use parking_lot::Mutex;

/// In-memory `audit_log` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryAuditStore {
    inner: Arc<Mutex<Vec<AuditLogRow>>>,
}

impl InMemoryAuditStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, row: AuditLogRow) -> Result<(), StorageError> {
        self.inner.lock().push(row);
        Ok(())
    }

    async fn list_for_org(
        &self,
        org_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditLogRow>, StorageError> {
        let mut rows: Vec<AuditLogRow> = self
            .inner
            .lock()
            .iter()
            .filter(|row| row.org_id == org_id)
            .cloned()
            .collect();
        // Newest first: emitted_at descending, ULID id as the tiebreaker —
        // the SQL `ORDER BY emitted_at DESC, id DESC` order.
        rows.sort_by(|a, b| {
            b.emitted_at
                .cmp(&a.emitted_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(rows)
    }
}
