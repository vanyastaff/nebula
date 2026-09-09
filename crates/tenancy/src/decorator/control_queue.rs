//! Scope-enforcing [`ControlQueue`] decorator.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage_port::dto::ControlMsg;
use nebula_storage_port::store::{ControlClaim, ControlClaimToken, ControlQueue, ReclaimOutcome};
use nebula_storage_port::{Scope, StorageError};

/// Wraps a [`ControlQueue`] and forces every `enqueue` into the bound
/// [`Scope`].
///
/// `enqueue` overwrites `msg.scope` with the bound scope: a low-privilege
/// tenant cannot enqueue a Cancel/Terminate carrying another tenant's
/// scope (§6.1 control-queue confused-deputy). The consumer-side methods
/// Claiming and maintenance remain worker-wide operations. Acknowledgement
/// rejects tokens bound to another tenant before they reach storage, so a
/// caller cannot use a token-shaped value to probe or mutate another tenant's
/// row.
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

    fn admit_claim<'a>(
        &self,
        claim: &'a ControlClaimToken,
    ) -> Result<&'a ControlClaimToken, StorageError> {
        if claim.scope() == &self.bound {
            Ok(claim)
        } else {
            Err(StorageError::NotFound {
                entity: "control_queue",
                id: "scoped-claim".to_owned(),
            })
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
    ) -> Result<Vec<ControlClaim>, StorageError> {
        self.inner.claim_pending(processor, batch_size).await
    }

    async fn claim_pending_for_flavor(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        worker_flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        self.inner
            .claim_pending_for_flavor(processor, batch_size, worker_flavor)
            .await
    }

    async fn mark_completed(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        self.inner.mark_completed(self.admit_claim(claim)?).await
    }

    async fn mark_failed(
        &self,
        claim: &ControlClaimToken,
        error: &str,
    ) -> Result<(), StorageError> {
        self.inner
            .mark_failed(self.admit_claim(claim)?, error)
            .await
    }

    async fn release_claim(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        self.inner.release_claim(self.admit_claim(claim)?).await
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
