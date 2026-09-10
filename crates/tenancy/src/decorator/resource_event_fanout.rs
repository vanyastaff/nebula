//! Scope-substituting resource-event fanout decorator.

use std::sync::Arc;

use nebula_storage_port::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, ClaimResourceDeliveriesRequest,
    ClaimedResourceDelivery, CompleteResourceDeliveryOutcome, CompleteResourceDeliveryRequest,
    HeartbeatResourceDeliveryRequest, ReleaseResourceDeliveryRequest, ResourceEventId,
    ResourceEventRecord,
};
use nebula_storage_port::store::ResourceEventFanoutStore;
use nebula_storage_port::{Scope, StorageError};

/// Forces every [`ResourceEventFanoutStore`] operation into one bound [`Scope`].
#[derive(Clone)]
pub struct ScopedResourceEventFanoutStore {
    inner: Arc<dyn ResourceEventFanoutStore>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedResourceEventFanoutStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedResourceEventFanoutStore")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedResourceEventFanoutStore {
    /// Bind a raw port to the principal-resolved scope.
    #[must_use]
    pub fn new(inner: Arc<dyn ResourceEventFanoutStore>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }
}

#[async_trait::async_trait]
impl ResourceEventFanoutStore for ScopedResourceEventFanoutStore {
    async fn accept(
        &self,
        request: AcceptResourceEventRequest,
    ) -> Result<AcceptResourceEventOutcome, StorageError> {
        self.inner
            .accept(request.with_scope(self.bound.clone()))
            .await
    }

    async fn get_event(
        &self,
        _scope: &Scope,
        event_id: ResourceEventId,
    ) -> Result<Option<ResourceEventRecord>, StorageError> {
        self.inner.get_event(&self.bound, event_id).await
    }

    async fn claim_deliveries(
        &self,
        request: ClaimResourceDeliveriesRequest,
    ) -> Result<Vec<ClaimedResourceDelivery>, StorageError> {
        self.inner
            .claim_deliveries(request.with_scope(self.bound.clone()))
            .await
    }

    async fn heartbeat_delivery(
        &self,
        request: HeartbeatResourceDeliveryRequest,
    ) -> Result<ClaimedResourceDelivery, StorageError> {
        self.inner
            .heartbeat_delivery(request.with_scope(self.bound.clone()))
            .await
    }

    async fn release_delivery(
        &self,
        request: ReleaseResourceDeliveryRequest,
    ) -> Result<(), StorageError> {
        self.inner
            .release_delivery(request.with_scope(self.bound.clone()))
            .await
    }

    async fn complete_delivery(
        &self,
        request: CompleteResourceDeliveryRequest,
    ) -> Result<CompleteResourceDeliveryOutcome, StorageError> {
        self.inner
            .complete_delivery(request.with_scope(self.bound.clone()))
            .await
    }
}
