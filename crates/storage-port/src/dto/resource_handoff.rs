//! Durable resource-owned execution-start handoff values.

use std::fmt;

use uuid::Uuid;

use crate::Scope;

use super::{
    EventEnvelope, ResourceDeliveryId, ResourceEventId, ResourceLeaseGeneration,
    ResourceLeaseHolder, ResourceLeaseTtl, ResourcePageSize, ResourceSubscriptionId,
};

/// Exact claim proof for one execution-start handoff.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceHandoffClaimToken {
    claim_id: Uuid,
    generation: ResourceLeaseGeneration,
}

impl ResourceHandoffClaimToken {
    /// Construct a store-minted claim token.
    pub const fn new(claim_id: Uuid, generation: ResourceLeaseGeneration) -> Self {
        Self {
            claim_id,
            generation,
        }
    }

    /// Rehydrate a claim token from persisted bytes.
    pub const fn from_claim_bytes(claim_id: [u8; 16], generation: ResourceLeaseGeneration) -> Self {
        Self::new(Uuid::from_bytes(claim_id), generation)
    }

    /// Return the opaque claim identifier.
    pub const fn claim_id(&self) -> Uuid {
        self.claim_id
    }

    /// Return the fencing generation.
    pub const fn generation(&self) -> ResourceLeaseGeneration {
        self.generation
    }
}

impl fmt::Debug for ResourceHandoffClaimToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResourceHandoffClaimToken([opaque])")
    }
}

/// A claimed execution-start handoff carrying the exact accepted envelope.
#[derive(Clone, PartialEq, Eq)]
pub struct ClaimedResourceHandoff {
    delivery_id: ResourceDeliveryId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
    envelope: EventEnvelope,
    token: ResourceHandoffClaimToken,
}

impl ClaimedResourceHandoff {
    /// Rehydrate a claimed handoff at a persistence boundary.
    pub const fn new(
        delivery_id: ResourceDeliveryId,
        event_id: ResourceEventId,
        subscription_id: ResourceSubscriptionId,
        envelope: EventEnvelope,
        token: ResourceHandoffClaimToken,
    ) -> Self {
        Self {
            delivery_id,
            event_id,
            subscription_id,
            envelope,
            token,
        }
    }

    /// Return the delivery that uniquely keys this handoff.
    pub const fn delivery_id(&self) -> ResourceDeliveryId {
        self.delivery_id
    }

    /// Return the accepted event.
    pub const fn event_id(&self) -> ResourceEventId {
        self.event_id
    }

    /// Return the snapshotted subscription.
    pub const fn subscription_id(&self) -> ResourceSubscriptionId {
        self.subscription_id
    }

    /// Return the exact accepted envelope.
    pub const fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }

    /// Return the exact handoff claim proof.
    pub const fn token(&self) -> &ResourceHandoffClaimToken {
        &self.token
    }
}

impl fmt::Debug for ClaimedResourceHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimedResourceHandoff")
            .field("delivery_id", &self.delivery_id)
            .field("event_id", &self.event_id)
            .field("subscription_id", &self.subscription_id)
            .field("envelope", &self.envelope)
            .field("token", &self.token)
            .finish()
    }
}

/// Bounded request to claim recoverable execution-start handoffs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimResourceHandoffsRequest {
    scope: Scope,
    holder: ResourceLeaseHolder,
    ttl: ResourceLeaseTtl,
    batch_size: ResourcePageSize,
}

impl ClaimResourceHandoffsRequest {
    /// Construct a bounded handoff claim request.
    pub const fn new(
        scope: Scope,
        holder: ResourceLeaseHolder,
        ttl: ResourceLeaseTtl,
        batch_size: ResourcePageSize,
    ) -> Self {
        Self {
            scope,
            holder,
            ttl,
            batch_size,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the claimant identity.
    pub const fn holder(&self) -> &ResourceLeaseHolder {
        &self.holder
    }

    /// Return the claim TTL.
    pub const fn ttl(&self) -> ResourceLeaseTtl {
        self.ttl
    }

    /// Return the bounded maximum result count.
    pub const fn batch_size(&self) -> ResourcePageSize {
        self.batch_size
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Request concerning one exact handoff claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceHandoffClaimRequest {
    scope: Scope,
    delivery_id: ResourceDeliveryId,
    token: ResourceHandoffClaimToken,
}

impl ResourceHandoffClaimRequest {
    /// Construct an exact handoff claim request.
    pub const fn new(
        scope: Scope,
        delivery_id: ResourceDeliveryId,
        token: ResourceHandoffClaimToken,
    ) -> Self {
        Self {
            scope,
            delivery_id,
            token,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the delivery key.
    pub const fn delivery_id(&self) -> ResourceDeliveryId {
        self.delivery_id
    }

    /// Return the exact claim proof.
    pub const fn token(&self) -> &ResourceHandoffClaimToken {
        &self.token
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Heartbeat request for one exact handoff claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResourceHandoffRequest {
    claim: ResourceHandoffClaimRequest,
    ttl: ResourceLeaseTtl,
}

impl HeartbeatResourceHandoffRequest {
    /// Construct a handoff heartbeat request.
    pub const fn new(claim: ResourceHandoffClaimRequest, ttl: ResourceLeaseTtl) -> Self {
        Self { claim, ttl }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        self.claim.scope()
    }

    /// Return the delivery key.
    pub const fn delivery_id(&self) -> ResourceDeliveryId {
        self.claim.delivery_id()
    }

    /// Return the exact claim proof.
    pub const fn token(&self) -> &ResourceHandoffClaimToken {
        self.claim.token()
    }

    /// Return the replacement TTL.
    pub const fn ttl(&self) -> ResourceLeaseTtl {
        self.ttl
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.claim = self.claim.with_scope(scope);
        self
    }
}

/// Idempotent result of acknowledging an execution-start handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcknowledgeResourceHandoffOutcome {
    /// This call consumed the handoff.
    Acknowledged,
    /// The same exact claim had already consumed the handoff.
    AlreadyAcknowledged,
}
