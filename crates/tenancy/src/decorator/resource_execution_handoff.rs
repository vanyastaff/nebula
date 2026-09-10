//! Scope-substituting resource execution-handoff store decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{
    AcknowledgeResourceHandoffOutcome, ClaimResourceHandoffsRequest, ClaimedResourceHandoff,
    HeartbeatResourceHandoffRequest, ResourceHandoffClaimRequest,
};
use nebula_storage_port::store::ResourceExecutionHandoffStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every [`ResourceExecutionHandoffStore`] operation into one bound [`Scope`].
#[derive(Clone)]
pub struct ScopedResourceExecutionHandoffStore {
    inner: Arc<dyn ResourceExecutionHandoffStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedResourceExecutionHandoffStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedResourceExecutionHandoffStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedResourceExecutionHandoffStore {
    /// Bind a raw port to the principal-resolved scope.
    #[must_use]
    pub fn new(inner: Arc<dyn ResourceExecutionHandoffStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ResourceExecutionHandoffStore for ScopedResourceExecutionHandoffStore {
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError> {
        self.inner
            .claim_handoffs(request.with_scope(self.bound.clone()))
            .await
    }

    async fn heartbeat_handoff(
        &self,
        request: HeartbeatResourceHandoffRequest,
    ) -> Result<ClaimedResourceHandoff, StorageError> {
        self.inner
            .heartbeat_handoff(request.with_scope(self.bound.clone()))
            .await
    }

    async fn release_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<(), StorageError> {
        self.inner
            .release_handoff(request.with_scope(self.bound.clone()))
            .await
    }

    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<AcknowledgeResourceHandoffOutcome, StorageError> {
        self.inner
            .acknowledge_handoff(request.with_scope(self.bound.clone()))
            .await
    }
}
