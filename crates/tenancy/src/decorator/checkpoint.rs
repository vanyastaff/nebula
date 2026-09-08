//! Scope-enforcing [`CheckpointStore`] decorator.

use std::sync::Arc;

use nebula_storage_port::store::CheckpointStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every checkpoint read and write into one bound tenant.
#[derive(Clone)]
pub struct ScopedCheckpointStore {
    inner: Arc<dyn CheckpointStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedCheckpointStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedCheckpointStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedCheckpointStore {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn CheckpointStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl CheckpointStore for ScopedCheckpointStore {
    async fn save_stateful_checkpoint(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
        checkpoint: serde_json::Value,
    ) -> Result<(), StorageError> {
        self.inner
            .save_stateful_checkpoint(&self.bound, execution_id, node_id, checkpoint)
            .await
    }

    async fn load_stateful_checkpoint(
        &self,
        _scope: &Scope,
        execution_id: &str,
        node_id: &str,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        self.inner
            .load_stateful_checkpoint(&self.bound, execution_id, node_id)
            .await
    }
}
