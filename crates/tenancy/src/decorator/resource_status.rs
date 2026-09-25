//! Scope-substituting resource-status store decorator.

use std::{sync::Arc, time::Duration};

use nebula_storage_port::dto::{LiveResourceStatus, ResourceStatusSnapshot, StatusWorkerId};
use nebula_storage_port::store::ResourceStatusStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every tenant-keyed [`ResourceStatusStore`] operation into one bound
/// [`Scope`].
///
/// Worker lifecycle — [`heartbeat`](ResourceStatusStore::heartbeat) and
/// [`withdraw_worker`](ResourceStatusStore::withdraw_worker) — spans every
/// tenant a worker serves, so a tenant-bound handle refuses it with
/// [`StorageError::ScopeViolation`]: only the worker's composition root, which
/// holds the raw store, may mark a worker live or withdraw all it published.
#[derive(Clone)]
pub struct ScopedResourceStatusStore {
    inner: Arc<dyn ResourceStatusStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedResourceStatusStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedResourceStatusStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedResourceStatusStore {
    /// Bind a raw port to the principal-resolved scope.
    #[must_use]
    pub fn new(inner: Arc<dyn ResourceStatusStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

const ENTITY: &str = "resource_status";

#[async_trait::async_trait]
impl ResourceStatusStore for ScopedResourceStatusStore {
    async fn heartbeat(
        &self,
        _worker: &StatusWorkerId,
        _ttl: Duration,
    ) -> Result<(), StorageError> {
        Err(StorageError::ScopeViolation { entity: ENTITY })
    }

    async fn publish(
        &self,
        _scope: &Scope,
        worker: &StatusWorkerId,
        snapshot: &ResourceStatusSnapshot,
    ) -> Result<(), StorageError> {
        self.inner.publish(&self.bound, worker, snapshot).await
    }

    async fn withdraw(
        &self,
        _scope: &Scope,
        worker: &StatusWorkerId,
        resource_id: &str,
    ) -> Result<(), StorageError> {
        self.inner.withdraw(&self.bound, worker, resource_id).await
    }

    async fn withdraw_worker(&self, _worker: &StatusWorkerId) -> Result<(), StorageError> {
        Err(StorageError::ScopeViolation { entity: ENTITY })
    }

    async fn live_for(
        &self,
        _scope: &Scope,
        resource_id: &str,
    ) -> Result<Vec<LiveResourceStatus>, StorageError> {
        self.inner.live_for(&self.bound, resource_id).await
    }
}
