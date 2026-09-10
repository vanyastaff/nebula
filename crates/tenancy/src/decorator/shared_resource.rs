//! Scope-substituting shared-resource store decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{
    ReconciliationCursor, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourcePageSize, SharedResourceId, SharedResourcePage, SharedResourceRecord,
};
use nebula_storage_port::store::SharedResourceStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every [`SharedResourceStore`] operation into one bound [`Scope`].
#[derive(Clone)]
pub struct ScopedSharedResourceStore {
    inner: Arc<dyn SharedResourceStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedSharedResourceStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedSharedResourceStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedSharedResourceStore {
    /// Bind a raw port to the principal-resolved scope.
    #[must_use]
    pub fn new(inner: Arc<dyn SharedResourceStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl SharedResourceStore for ScopedSharedResourceStore {
    async fn resolve(
        &self,
        request: ResolveSharedResourceRequest,
    ) -> Result<ResolveSharedResourceOutcome, StorageError> {
        self.inner
            .resolve(request.with_scope(self.bound.clone()))
            .await
    }

    async fn get(
        &self,
        _scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<Option<SharedResourceRecord>, StorageError> {
        self.inner.get(&self.bound, resource_id).await
    }

    async fn list_for_reconciliation(
        &self,
        _scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<SharedResourcePage, StorageError> {
        self.inner
            .list_for_reconciliation(&self.bound, after, page_size)
            .await
    }
}
