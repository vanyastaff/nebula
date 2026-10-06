//! Quotas: per-org limits and the concurrent-execution counter.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::QuotaRow;
use nebula_storage_port::store::QuotaStore;
use parking_lot::Mutex;

/// In-memory `org_quotas` + `org_quota_usage` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryQuotaStore {
    inner: Arc<Mutex<HashMap<String, QuotaRow>>>,
}

impl InMemoryQuotaStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl QuotaStore for InMemoryQuotaStore {
    async fn get(&self, org_id: &str) -> Result<Option<QuotaRow>, StorageError> {
        Ok(self.inner.lock().get(org_id).cloned())
    }

    async fn upsert(&self, row: QuotaRow) -> Result<(), StorageError> {
        self.inner.lock().insert(row.org_id.clone(), row);
        Ok(())
    }

    async fn adjust_concurrent(&self, org_id: &str, delta: i32) -> Result<i32, StorageError> {
        let mut rows = self.inner.lock();
        let Some(row) = rows.get_mut(org_id) else {
            return Err(StorageError::not_found("quota", org_id));
        };
        let next = row
            .concurrent_executions
            .checked_add(delta)
            .ok_or_else(|| {
                StorageError::InvalidInput("concurrent execution adjustment overflows".into())
            })?;
        if next < 0 {
            return Err(StorageError::Conflict {
                entity: "quota",
                id: org_id.to_owned(),
                expected: 0,
                actual: u64::from(row.concurrent_executions.unsigned_abs()),
            });
        }
        row.concurrent_executions = next;
        Ok(next)
    }
}
