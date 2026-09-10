//! Scope-substituting resource-subscription store decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{
    PutResourceSubscriptionOutcome, PutResourceSubscriptionRequest, ReconciliationCursor,
    ResourcePageSize, ResourceSubscriptionId, ResourceSubscriptionPage, ResourceSubscriptionRecord,
    SharedResourceId, TransitionResourceSubscriptionRequest,
};
use nebula_storage_port::store::ResourceSubscriptionStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every [`ResourceSubscriptionStore`] operation into one bound [`Scope`].
#[derive(Clone)]
pub struct ScopedResourceSubscriptionStore {
    inner: Arc<dyn ResourceSubscriptionStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedResourceSubscriptionStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedResourceSubscriptionStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedResourceSubscriptionStore {
    /// Bind a raw port to the principal-resolved scope.
    #[must_use]
    pub fn new(inner: Arc<dyn ResourceSubscriptionStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ResourceSubscriptionStore for ScopedResourceSubscriptionStore {
    async fn put(
        &self,
        request: PutResourceSubscriptionRequest,
    ) -> Result<PutResourceSubscriptionOutcome, StorageError> {
        self.inner.put(request.with_scope(self.bound.clone())).await
    }

    async fn get(
        &self,
        _scope: &Scope,
        subscription_id: ResourceSubscriptionId,
    ) -> Result<Option<ResourceSubscriptionRecord>, StorageError> {
        self.inner.get(&self.bound, subscription_id).await
    }

    async fn list_active_for_resource(
        &self,
        _scope: &Scope,
        resource_id: SharedResourceId,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError> {
        self.inner
            .list_active_for_resource(&self.bound, resource_id, after, page_size)
            .await
    }

    async fn list_for_reconciliation(
        &self,
        _scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError> {
        self.inner
            .list_for_reconciliation(&self.bound, after, page_size)
            .await
    }

    async fn count_active_for_resource(
        &self,
        _scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<u64, StorageError> {
        self.inner
            .count_active_for_resource(&self.bound, resource_id)
            .await
    }

    async fn transition(
        &self,
        request: TransitionResourceSubscriptionRequest,
    ) -> Result<ResourceSubscriptionRecord, StorageError> {
        self.inner
            .transition(request.with_scope(self.bound.clone()))
            .await
    }
}
