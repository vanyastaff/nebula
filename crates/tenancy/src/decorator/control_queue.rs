//! Scope-enforcing [`ControlQueue`] decorator.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::ControlMsg;
use nebula_storage_port::store::{ControlQueue, ReclaimOutcome};
use nebula_storage_port::{Scope, StorageError};

/// Wraps a [`ControlQueue`] and forces every `enqueue` into the bound
/// [`Scope`].
///
/// `enqueue` overwrites `msg.scope` with the bound scope: a low-privilege
/// tenant cannot enqueue a Cancel/Terminate carrying another tenant's
/// scope (§6.1 control-queue confused-deputy). The consumer-side methods
/// (`claim_pending`, `mark_*`, `reclaim_stuck`, `cleanup`) are
/// deliberately *not* scoped — claiming is a cross-tenant worker
/// operation; the engine consumer re-verifies scope against the execution
/// row before dispatch (spec §6.1 point 3). They forward unchanged.
#[derive(Clone)]
pub struct ScopedControlQueue {
    inner: Arc<dyn ControlQueue>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedControlQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedControlQueue")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedControlQueue {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn ControlQueue>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ControlQueue for ScopedControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        let mut scoped = msg.clone();
        scoped.scope = self.bound.clone();
        self.inner.enqueue(&scoped).await
    }

    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlMsg>, StorageError> {
        self.inner.claim_pending(processor, batch_size).await
    }

    async fn mark_completed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        self.inner.mark_completed(id, processor).await
    }

    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError> {
        self.inner.mark_failed(id, processor, error).await
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        self.inner
            .reclaim_stuck(reclaim_after, max_reclaim_count)
            .await
    }

    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError> {
        self.inner.cleanup(retention).await
    }
}
