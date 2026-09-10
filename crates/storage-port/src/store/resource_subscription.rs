//! Object-safe persistence roles for shared resources, subscriptions, leases, and fanout.

use crate::dto::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, AcknowledgeResourceHandoffOutcome,
    AcquireResourceSourceLeaseOutcome, AcquireResourceSourceLeaseRequest,
    ClaimResourceDeliveriesRequest, ClaimResourceHandoffsRequest, ClaimResourceRuntimeWorkRequest,
    ClaimedResourceDelivery, ClaimedResourceHandoff, CompleteResourceDeliveryOutcome,
    CompleteResourceDeliveryRequest, HeartbeatResourceDeliveryRequest,
    HeartbeatResourceHandoffRequest, HeartbeatResourceSourceLeaseRequest,
    PutResourceSubscriptionOutcome, PutResourceSubscriptionRequest, ReconciliationCursor,
    ReleaseResourceDeliveryRequest, ReleaseResourceSourceLeaseRequest,
    ResolveSharedResourceOutcome, ResolveSharedResourceRequest, ResourceEventId,
    ResourceEventRecord, ResourceHandoffClaimRequest, ResourcePageSize, ResourceSourceLease,
    ResourceSubscriptionId, ResourceSubscriptionPage, ResourceSubscriptionRecord,
    ScopedClaimedResourceDelivery, ScopedClaimedResourceHandoff, SharedResourceId,
    SharedResourcePage, SharedResourceRecord, TransitionResourceSubscriptionRequest,
};
use crate::{Scope, StorageError};

/// Exact identity and reconciliation role for shared runtime resources.
#[async_trait::async_trait]
pub trait SharedResourceStore: Send + Sync + 'static {
    /// Resolve an exact scoped identity, atomically creating it when absent.
    async fn resolve(
        &self,
        request: ResolveSharedResourceRequest,
    ) -> Result<ResolveSharedResourceOutcome, StorageError>;

    /// Load one resource only from the supplied scope.
    async fn get(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<Option<SharedResourceRecord>, StorageError>;

    /// Load an increasing-sequence page strictly after `after`.
    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<SharedResourcePage, StorageError>;
}

/// Durable consumer-subscription lifecycle role.
#[async_trait::async_trait]
pub trait ResourceSubscriptionStore: Send + Sync + 'static {
    /// Resolve an exact resource/consumer binding, creating it Active when absent.
    async fn put(
        &self,
        request: PutResourceSubscriptionRequest,
    ) -> Result<PutResourceSubscriptionOutcome, StorageError>;

    /// Load one subscription only from the supplied scope.
    async fn get(
        &self,
        scope: &Scope,
        subscription_id: ResourceSubscriptionId,
    ) -> Result<Option<ResourceSubscriptionRecord>, StorageError>;

    /// List one stable page of Active subscriptions for a resource.
    async fn list_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError>;

    /// List one stable page of all subscriptions for restart reconciliation.
    async fn list_for_reconciliation(
        &self,
        scope: &Scope,
        after: Option<ReconciliationCursor>,
        page_size: ResourcePageSize,
    ) -> Result<ResourceSubscriptionPage, StorageError>;

    /// Count Active subscription demand for one resource.
    async fn count_active_for_resource(
        &self,
        scope: &Scope,
        resource_id: SharedResourceId,
    ) -> Result<u64, StorageError>;

    /// Apply one optimistic-CAS lifecycle transition.
    async fn transition(
        &self,
        request: TransitionResourceSubscriptionRequest,
    ) -> Result<ResourceSubscriptionRecord, StorageError>;
}

/// Recovery and claiming role for resource-owned execution-start handoffs.
#[async_trait::async_trait]
pub trait ResourceExecutionHandoffStore: Send + Sync + 'static {
    /// Claim a bounded sequence-ordered batch, taking over at exact expiry.
    async fn claim_handoffs(
        &self,
        request: ClaimResourceHandoffsRequest,
    ) -> Result<Vec<ClaimedResourceHandoff>, StorageError>;

    /// Extend only one live exact handoff claim.
    async fn heartbeat_handoff(
        &self,
        request: HeartbeatResourceHandoffRequest,
    ) -> Result<ClaimedResourceHandoff, StorageError>;

    /// Release one exact handoff claim idempotently.
    async fn release_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<(), StorageError>;

    /// Acknowledge one live exact claim, idempotently for the same token.
    async fn acknowledge_handoff(
        &self,
        request: ResourceHandoffClaimRequest,
    ) -> Result<AcknowledgeResourceHandoffOutcome, StorageError>;
}

/// Deployment-only recovery role spanning every persisted tenant scope.
///
/// Tenant-facing services use scoped roles. This capability is deliberately
/// not tenancy-decorated; every result carries its authoritative stored scope.
#[async_trait::async_trait]
pub trait ResourceRuntimeRecovery: Send + Sync + 'static {
    /// Claim a globally sequence-ordered batch of recoverable deliveries.
    async fn claim_deliveries_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceDelivery>, StorageError>;

    /// Claim a globally sequence-ordered batch of recoverable handoffs.
    async fn claim_handoffs_globally(
        &self,
        request: ClaimResourceRuntimeWorkRequest,
    ) -> Result<Vec<ScopedClaimedResourceHandoff>, StorageError>;
}

/// Atomic source ownership role for one shared resource.
#[async_trait::async_trait]
pub trait ResourceSourceLeaseStore: Send + Sync + 'static {
    /// Acquire a missing lease or atomically take over a lease at exact expiry.
    ///
    /// Generation exhaustion fails closed with a store error; it never wraps.
    async fn acquire(
        &self,
        request: AcquireResourceSourceLeaseRequest,
    ) -> Result<AcquireResourceSourceLeaseOutcome, StorageError>;

    /// Extend only a live exact token, replacing expiry with backend-now plus TTL.
    async fn heartbeat(
        &self,
        request: HeartbeatResourceSourceLeaseRequest,
    ) -> Result<ResourceSourceLease, StorageError>;

    /// Release the exact token idempotently, including after its expiry.
    async fn release(&self, request: ReleaseResourceSourceLeaseRequest)
    -> Result<(), StorageError>;
}

/// Atomic event acceptance and durable per-subscription fanout role.
#[async_trait::async_trait]
pub trait ResourceEventFanoutStore: Send + Sync + 'static {
    /// Accept an event under a live exact source token and atomically snapshot
    /// every Active subscription into one delivery each.
    ///
    /// Deduplication is scoped by scope, resource, namespace, and occurrence
    /// key. Replay requires the same schema and every exact envelope byte;
    /// digest equality alone never proves replay.
    async fn accept(
        &self,
        request: AcceptResourceEventRequest,
    ) -> Result<AcceptResourceEventOutcome, StorageError>;

    /// Load one accepted event only from the supplied scope.
    async fn get_event(
        &self,
        scope: &Scope,
        event_id: ResourceEventId,
    ) -> Result<Option<ResourceEventRecord>, StorageError>;

    /// Atomically claim up to the bounded batch size, taking over claims at exact expiry.
    async fn claim_deliveries(
        &self,
        request: ClaimResourceDeliveriesRequest,
    ) -> Result<Vec<ClaimedResourceDelivery>, StorageError>;

    /// Extend only one live exact delivery claim.
    async fn heartbeat_delivery(
        &self,
        request: HeartbeatResourceDeliveryRequest,
    ) -> Result<ClaimedResourceDelivery, StorageError>;

    /// Release one exact delivery claim idempotently.
    async fn release_delivery(
        &self,
        request: ReleaseResourceDeliveryRequest,
    ) -> Result<(), StorageError>;

    /// Terminalize one exact claim and atomically terminalize its parent event
    /// when no non-terminal deliveries remain.
    async fn complete_delivery(
        &self,
        request: CompleteResourceDeliveryRequest,
    ) -> Result<CompleteResourceDeliveryOutcome, StorageError>;
}
