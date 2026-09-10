//! Deployment-wide recovery values for durable resource fanout.

use crate::Scope;

use super::{
    ClaimedResourceDelivery, ClaimedResourceHandoff, ResourceLeaseHolder, ResourceLeaseTtl,
    ResourcePageSize,
};

/// Scope-free bounded request used only by a deployment runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimResourceRuntimeWorkRequest {
    holder: ResourceLeaseHolder,
    ttl: ResourceLeaseTtl,
    batch_size: ResourcePageSize,
}

impl ClaimResourceRuntimeWorkRequest {
    /// Construct a deployment-wide recovery claim request.
    pub const fn new(
        holder: ResourceLeaseHolder,
        ttl: ResourceLeaseTtl,
        batch_size: ResourcePageSize,
    ) -> Self {
        Self {
            holder,
            ttl,
            batch_size,
        }
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
}

/// A globally claimed delivery paired with its authoritative persisted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedClaimedResourceDelivery {
    scope: Scope,
    delivery: ClaimedResourceDelivery,
}

impl ScopedClaimedResourceDelivery {
    /// Rehydrate a globally claimed delivery at a persistence boundary.
    pub const fn new(scope: Scope, delivery: ClaimedResourceDelivery) -> Self {
        Self { scope, delivery }
    }

    /// Return the authoritative scope read with the claim.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the claimed delivery.
    pub const fn delivery(&self) -> &ClaimedResourceDelivery {
        &self.delivery
    }

    /// Consume the wrapper into its authoritative scope and claim.
    pub fn into_parts(self) -> (Scope, ClaimedResourceDelivery) {
        (self.scope, self.delivery)
    }
}

/// A globally claimed handoff paired with its authoritative persisted scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedClaimedResourceHandoff {
    scope: Scope,
    handoff: ClaimedResourceHandoff,
}

impl ScopedClaimedResourceHandoff {
    /// Rehydrate a globally claimed handoff at a persistence boundary.
    pub const fn new(scope: Scope, handoff: ClaimedResourceHandoff) -> Self {
        Self { scope, handoff }
    }

    /// Return the authoritative scope read with the claim.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the claimed handoff.
    pub const fn handoff(&self) -> &ClaimedResourceHandoff {
        &self.handoff
    }

    /// Consume the wrapper into its authoritative scope and claim.
    pub fn into_parts(self) -> (Scope, ClaimedResourceHandoff) {
        (self.scope, self.handoff)
    }
}
