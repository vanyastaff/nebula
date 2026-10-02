//! Scope-enforcing [`CheckpointStore`] decorator.

use std::sync::Arc;

use nebula_storage_port::store::CheckpointStore;
use nebula_storage_port::{
    CheckpointSaved, FencingToken, IterationCheckpoint, IterationCheckpointError,
    IterationCheckpointKey, Scope,
};

/// Forces every checkpoint read and write into one bound tenant.
///
/// The caller's key is re-addressed under the bound scope; the fencing token
/// passes through unchanged, so the backend still fences the write by the
/// bound tenant's execution lease.
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
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
        self.inner
            .load_iteration_checkpoint(&key.rescoped(&self.bound))
            .await
    }

    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError> {
        self.inner
            .save_iteration_checkpoint(&key.rescoped(&self.bound), checkpoint, fencing)
            .await
    }
}
