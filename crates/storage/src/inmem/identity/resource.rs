//! Resources: workspace-scoped; slug is unique among active rows per
//! workspace scope.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::dto::ResourceRow;
use nebula_storage_port::store::ResourceStore;
use nebula_storage_port::{Scope, StorageError};
use parking_lot::Mutex;

use super::{ScopedKey, duplicate, in_scope, now_rfc3339, scoped_key, version_conflict};

/// In-memory `resources` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryResourceStore {
    inner: Arc<Mutex<HashMap<ScopedKey, ResourceRow>>>,
}

impl InMemoryResourceStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

fn slug_taken(rows: &HashMap<ScopedKey, ResourceRow>, scope: &Scope, row: &ResourceRow) -> bool {
    rows.iter().any(|(key, other)| {
        in_scope(key, scope)
            && key.2 != row.id
            && other.deleted_at.is_none()
            && other.slug == row.slug
    })
}

#[async_trait::async_trait]
impl ResourceStore for InMemoryResourceStore {
    async fn create(&self, scope: &Scope, row: ResourceRow) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut rows = self.inner.lock();
        if rows.contains_key(&key) {
            return Err(duplicate("resource", "id"));
        }
        if slug_taken(&rows, scope, &row) {
            return Err(duplicate("resource", "slug"));
        }
        rows.insert(key, row);
        Ok(())
    }

    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ResourceRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&scoped_key(scope, id))
            .filter(|row| row.deleted_at.is_none())
            .cloned())
    }

    async fn list(&self, scope: &Scope) -> Result<Vec<ResourceRow>, StorageError> {
        let mut rows: Vec<ResourceRow> = self
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
        row: ResourceRow,
        expected_version: u64,
    ) -> Result<(), StorageError> {
        let key = scoped_key(scope, &row.id);
        let mut rows = self.inner.lock();
        let Some(current) = rows.get(&key).filter(|row| row.deleted_at.is_none()) else {
            return Err(StorageError::not_found("resource", row.id));
        };
        if current.version != expected_version {
            let actual = current.version;
            return Err(version_conflict(
                "resource",
                row.id,
                expected_version,
                actual,
            ));
        }
        if slug_taken(&rows, scope, &row) {
            return Err(duplicate("resource", "slug"));
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
            return Err(StorageError::not_found("resource", id));
        };
        row.deleted_at = Some(now_rfc3339());
        Ok(())
    }
}
