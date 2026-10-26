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
/// scope (§6.1 control-queue confused-deputy). Acknowledgement rejects
/// tokens bound to another tenant before they reach storage, so a caller
/// cannot use a token-shaped value to probe or mutate another tenant's row.
///
/// Claiming ([`claim_pending`](ControlQueue::claim_pending) and its flavor
/// variants) and maintenance ([`reclaim_stuck`](ControlQueue::reclaim_stuck),
/// [`cleanup`](ControlQueue::cleanup)) take no scope: they read and rewrite
/// every tenant's rows. They are worker-global, so a tenant-bound handle
/// refuses them with [`StorageError::ScopeViolation`] without reaching
/// storage. Only the worker's composition root, which holds the raw queue,
/// runs the consumer and its sweeps.
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

    fn refuse_worker_global<T>() -> Result<T, StorageError> {
        Err(StorageError::ScopeViolation { entity: ENTITY })
    }

    fn admit_claim<'a>(
        &self,
        claim: &'a ControlClaimToken,
    ) -> Result<&'a ControlClaimToken, StorageError> {
        if claim.scope() == &self.bound {
            Ok(claim)
        } else {
            Err(StorageError::NotFound {
                entity: ENTITY,
                id: "scoped-claim".to_owned(),
            })
        }
    }
}

const ENTITY: &str = "control_queue";

#[async_trait::async_trait]
impl ControlQueue for ScopedControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        let mut scoped = msg.clone();
        scoped.scope = self.bound.clone();
        self.inner.enqueue(&scoped).await
    }

    /// Worker-global: refused on a tenant-bound handle.
    async fn claim_pending(
        &self,
        _processor: &[u8; 16],
        _batch_size: u32,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        Self::refuse_worker_global()
    }

    /// Worker-global: refused on a tenant-bound handle.
    async fn claim_pending_for_flavor(
        &self,
        _processor: &[u8; 16],
        _batch_size: u32,
        _worker_flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        Self::refuse_worker_global()
    }

    /// Worker-global: refused on a tenant-bound handle.
    async fn claim_pending_interrupts_for_flavor(
        &self,
        _processor: &[u8; 16],
        _batch_size: u32,
        _worker_flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        Self::refuse_worker_global()
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

    /// Worker-global: refused on a tenant-bound handle.
    async fn reclaim_stuck(
        &self,
        _reclaim_after: Duration,
        _max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        Self::refuse_worker_global()
    }

    /// Worker-global: refused on a tenant-bound handle.
    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        Self::refuse_worker_global()
    }
}
