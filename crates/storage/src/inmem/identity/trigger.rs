//! Triggers: workspace-scoped workflow triggers.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::TriggerRow;
use nebula_storage_port::store::TriggerStore;
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

use super::{ScopedKey, duplicate, in_scope, now_rfc3339, scoped_key, version_conflict};

/// In-memory `triggers` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryTriggerStore {
    inner: Arc<Mutex<HashMap<ScopedKey, TriggerRow>>>,
}

impl InMemoryTriggerStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl TriggerStore for InMemoryTriggerStore {
    async fn create(&self, scope: &Scope, row: TriggerRow) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut rows = self.inner.lock();
        if rows.contains_key(&key) {
            return Err(duplicate("trigger", "id"));
        }
        rows.insert(key, row);
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<TriggerRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&scoped_key(scope, id))
            .filter(|row| row.deleted_at.is_none())
            .cloned())
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<TriggerRow>, StorageError> {
        let mut rows: Vec<TriggerRow> = self
            .inner
            .lock()
            .iter()
            .filter(|(key, row)| in_scope(key, scope) && row.deleted_at.is_none())
            .map(|(_, row)| row.clone())
            .collect();
        // Same order as the SQL backends' `ORDER BY id`.
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows)
    }

    async fn update(
        &self,
        scope: &Scope,
        row: TriggerRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut rows = self.inner.lock();
        let Some(current) = rows.get(&key).filter(|row| row.deleted_at.is_none()) else {
            return Err(StorageError::not_found("trigger", row.id));
        };
        if current.version != expected_version {
            let actual = current.version;
            return Err(version_conflict(
                "trigger",
                row.id,
                expected_version,
                actual,
            ));
        }
        rows.insert(key, row);
        Ok(())
    }

    async fn soft_delete(&self, scope: &Scope, id: &str) -> Result<(), StorageError> {
        let mut rows = self.inner.lock();
        let Some(row) = rows
            .get_mut(&scoped_key(scope, id))
            .filter(|row| row.deleted_at.is_none())
        else {
            return Err(StorageError::not_found("trigger", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}
