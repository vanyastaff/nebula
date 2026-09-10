//! Scope-substituting shared-resource source-lease decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{
    AcquireResourceSourceLeaseOutcome, AcquireResourceSourceLeaseRequest,
    HeartbeatResourceSourceLeaseRequest, ReleaseResourceSourceLeaseRequest, ResourceSourceLease,
};
use nebula_storage_port::store::ResourceSourceLeaseStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every [`ResourceSourceLeaseStore`] operation into one bound [`Scope`].
#[derive(Clone)]
pub struct ScopedResourceSourceLeaseStore {
    inner: Arc<dyn ResourceSourceLeaseStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedResourceSourceLeaseStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedResourceSourceLeaseStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedResourceSourceLeaseStore {
    /// Bind a raw port to the principal-resolved scope.
    #[must_use]
    pub fn new(inner: Arc<dyn ResourceSourceLeaseStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ResourceSourceLeaseStore for ScopedResourceSourceLeaseStore {
    async fn acquire(
        &self,
        request: AcquireResourceSourceLeaseRequest,
    ) -> Result<AcquireResourceSourceLeaseOutcome, StorageError> {
        self.inner
            .acquire(request.with_scope(self.bound.clone()))
            .await
    }

    async fn heartbeat(
        &self,
        request: HeartbeatResourceSourceLeaseRequest,
    ) -> Result<ResourceSourceLease, StorageError> {
        self.inner
            .heartbeat(request.with_scope(self.bound.clone()))
            .await
    }

    async fn release(
        &self,
        request: ReleaseResourceSourceLeaseRequest,
    ) -> Result<(), StorageError> {
        self.inner
            .release(request.with_scope(self.bound.clone()))
            .await
    }
}
