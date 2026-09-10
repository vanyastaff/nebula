//! Source and delivery lease values for shared-resource runtime ownership.

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::Scope;

use super::{ResourceDeliveryId, ResourcePageSize, SharedResourceId};

const MAX_LEASE_HOLDER_BYTES: usize = 256;
const MIN_LEASE_TTL: Duration = Duration::from_secs(1);
const MAX_LEASE_TTL: Duration = Duration::from_hours(24);

/// Validation failure for a lease holder or TTL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResourceLeaseValueError {
    /// A holder was empty.
    #[error("lease holder must not be empty")]
    EmptyHolder,
    /// A holder exceeded 256 UTF-8 bytes.
    #[error("lease holder exceeds the 256-byte limit")]
    HolderTooLong,
    /// A TTL was outside `1 second..=24 hours`.
    #[error("lease TTL must be between 1 second and 24 hours")]
    InvalidTtl,
}

/// Bounded UTF-8 lease holder identity.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceLeaseHolder(String);

impl ResourceLeaseHolder {
    /// Construct a holder from `1..=256` UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceLeaseValueError`] when empty or too long.
    pub fn new(holder: impl Into<String>) -> Result<Self, ResourceLeaseValueError> {
        let holder = holder.into();
        if holder.is_empty() {
            return Err(ResourceLeaseValueError::EmptyHolder);
        }
        if holder.len() > MAX_LEASE_HOLDER_BYTES {
            return Err(ResourceLeaseValueError::HolderTooLong);
        }
        Ok(Self(holder))
    }

    /// Borrow the holder identity.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ResourceLeaseHolder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceLeaseHolder")
            .field("value", &self.0)
            .finish()
    }
}

/// Validated lease TTL in `1 second..=24 hours`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResourceLeaseTtl(Duration);

impl ResourceLeaseTtl {
    /// Construct a bounded lease TTL.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceLeaseValueError::InvalidTtl`] outside the accepted range.
    pub const fn new(ttl: Duration) -> Result<Self, ResourceLeaseValueError> {
        if ttl.as_nanos() < MIN_LEASE_TTL.as_nanos() || ttl.as_nanos() > MAX_LEASE_TTL.as_nanos() {
            return Err(ResourceLeaseValueError::InvalidTtl);
        }
        Ok(Self(ttl))
    }

    /// Return the validated duration.
    pub const fn get(self) -> Duration {
        self.0
    }
}

/// Generation of one exact source or delivery claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceLeaseGeneration(u64);

impl ResourceLeaseGeneration {
    /// Rehydrate a backend-authored generation.
    pub const fn new(generation: u64) -> Self {
        Self(generation)
    }

    /// Return the persisted generation.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Compute the next takeover generation, failing closed on overflow.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceLeaseGenerationOverflow`] at `u64::MAX`.
    pub const fn checked_next(self) -> Result<Self, ResourceLeaseGenerationOverflow> {
        match self.0.checked_add(1) {
            Some(next) => Ok(Self(next)),
            None => Err(ResourceLeaseGenerationOverflow),
        }
    }
}

/// Fail-closed lease generation overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("resource lease generation is exhausted")]
pub struct ResourceLeaseGenerationOverflow;

/// Exact source-lease ownership proof.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceSourceLeaseToken {
    claim_id: Uuid,
    generation: ResourceLeaseGeneration,
}

impl ResourceSourceLeaseToken {
    /// Rehydrate an exact source token at a persistence boundary.
    pub const fn new(claim_id: Uuid, generation: ResourceLeaseGeneration) -> Self {
        Self {
            claim_id,
            generation,
        }
    }

    /// Rehydrate an exact source token from persisted UUID bytes.
    pub const fn from_claim_bytes(claim_id: [u8; 16], generation: ResourceLeaseGeneration) -> Self {
        Self::new(Uuid::from_bytes(claim_id), generation)
    }

    /// Return the per-claim UUID.
    pub const fn claim_id(&self) -> Uuid {
        self.claim_id
    }

    /// Return the claim generation.
    pub const fn generation(&self) -> ResourceLeaseGeneration {
        self.generation
    }
}

impl fmt::Debug for ResourceSourceLeaseToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceSourceLeaseToken")
            .field("claim_id", &"[redacted]")
            .field("generation", &self.generation)
            .finish()
    }
}

/// Exact delivery-claim ownership proof.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceDeliveryClaimToken {
    claim_id: Uuid,
    generation: ResourceLeaseGeneration,
}

impl ResourceDeliveryClaimToken {
    /// Rehydrate an exact delivery token at a persistence boundary.
    pub const fn new(claim_id: Uuid, generation: ResourceLeaseGeneration) -> Self {
        Self {
            claim_id,
            generation,
        }
    }

    /// Rehydrate an exact delivery token from persisted UUID bytes.
    pub const fn from_claim_bytes(claim_id: [u8; 16], generation: ResourceLeaseGeneration) -> Self {
        Self::new(Uuid::from_bytes(claim_id), generation)
    }

    /// Return the per-claim UUID.
    pub const fn claim_id(&self) -> Uuid {
        self.claim_id
    }

    /// Return the claim generation.
    pub const fn generation(&self) -> ResourceLeaseGeneration {
        self.generation
    }
}

impl fmt::Debug for ResourceDeliveryClaimToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceDeliveryClaimToken")
            .field("claim_id", &"[redacted]")
            .field("generation", &self.generation)
            .finish()
    }
}

/// One live source lease returned by acquire or heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSourceLease {
    resource_id: SharedResourceId,
    holder: ResourceLeaseHolder,
    token: ResourceSourceLeaseToken,
    expires_at: DateTime<Utc>,
}

impl ResourceSourceLease {
    /// Rehydrate a live source lease.
    pub const fn new(
        resource_id: SharedResourceId,
        holder: ResourceLeaseHolder,
        token: ResourceSourceLeaseToken,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            resource_id,
            holder,
            token,
            expires_at,
        }
    }

    /// Return the claimed resource.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the holder identity.
    pub const fn holder(&self) -> &ResourceLeaseHolder {
        &self.holder
    }

    /// Return the exact ownership proof.
    pub const fn token(&self) -> &ResourceSourceLeaseToken {
        &self.token
    }

    /// Return the exact expiry instant. The lease is expired at this instant.
    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

/// Atomic acquire/takeover result for a source lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireResourceSourceLeaseOutcome {
    /// The caller acquired a missing or exactly expired lease.
    Acquired(ResourceSourceLease),
    /// Another exact token remains live.
    Contended {
        /// Backend-authored expiry used only as a retry hint.
        expires_at: DateTime<Utc>,
    },
}

/// Request to atomically acquire or take over a source lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireResourceSourceLeaseRequest {
    scope: Scope,
    resource_id: SharedResourceId,
    holder: ResourceLeaseHolder,
    ttl: ResourceLeaseTtl,
}

impl AcquireResourceSourceLeaseRequest {
    /// Construct an acquire request.
    pub const fn new(
        scope: Scope,
        resource_id: SharedResourceId,
        holder: ResourceLeaseHolder,
        ttl: ResourceLeaseTtl,
    ) -> Self {
        Self {
            scope,
            resource_id,
            holder,
            ttl,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the resource identifier.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the holder identity.
    pub const fn holder(&self) -> &ResourceLeaseHolder {
        &self.holder
    }

    /// Return the requested TTL.
    pub const fn ttl(&self) -> ResourceLeaseTtl {
        self.ttl
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Request to heartbeat an exact live source token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResourceSourceLeaseRequest {
    scope: Scope,
    resource_id: SharedResourceId,
    token: ResourceSourceLeaseToken,
    ttl: ResourceLeaseTtl,
}

impl HeartbeatResourceSourceLeaseRequest {
    /// Construct a source heartbeat request.
    pub const fn new(
        scope: Scope,
        resource_id: SharedResourceId,
        token: ResourceSourceLeaseToken,
        ttl: ResourceLeaseTtl,
    ) -> Self {
        Self {
            scope,
            resource_id,
            token,
            ttl,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the resource identifier.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the exact source token.
    pub const fn token(&self) -> &ResourceSourceLeaseToken {
        &self.token
    }

    /// Return the heartbeat TTL.
    pub const fn ttl(&self) -> ResourceLeaseTtl {
        self.ttl
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Request to release an exact source token idempotently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseResourceSourceLeaseRequest {
    scope: Scope,
    resource_id: SharedResourceId,
    token: ResourceSourceLeaseToken,
}

impl ReleaseResourceSourceLeaseRequest {
    /// Construct an exact-token source release request.
    pub const fn new(
        scope: Scope,
        resource_id: SharedResourceId,
        token: ResourceSourceLeaseToken,
    ) -> Self {
        Self {
            scope,
            resource_id,
            token,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the resource identifier.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the exact source token.
    pub const fn token(&self) -> &ResourceSourceLeaseToken {
        &self.token
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Request to claim a bounded batch of pending deliveries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimResourceDeliveriesRequest {
    scope: Scope,
    holder: ResourceLeaseHolder,
    ttl: ResourceLeaseTtl,
    batch_size: ResourcePageSize,
}

impl ClaimResourceDeliveriesRequest {
    /// Construct a delivery claim request.
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

    /// Return the holder identity.
    pub const fn holder(&self) -> &ResourceLeaseHolder {
        &self.holder
    }

    /// Return the requested TTL.
    pub const fn ttl(&self) -> ResourceLeaseTtl {
        self.ttl
    }

    /// Return the maximum claimed delivery count.
    pub const fn batch_size(&self) -> ResourcePageSize {
        self.batch_size
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Request to heartbeat an exact live delivery claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResourceDeliveryRequest {
    scope: Scope,
    delivery_id: ResourceDeliveryId,
    token: ResourceDeliveryClaimToken,
    ttl: ResourceLeaseTtl,
}

impl HeartbeatResourceDeliveryRequest {
    /// Construct a delivery heartbeat request.
    pub const fn new(
        scope: Scope,
        delivery_id: ResourceDeliveryId,
        token: ResourceDeliveryClaimToken,
        ttl: ResourceLeaseTtl,
    ) -> Self {
        Self {
            scope,
            delivery_id,
            token,
            ttl,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the delivery identifier.
    pub const fn delivery_id(&self) -> ResourceDeliveryId {
        self.delivery_id
    }

    /// Return the exact delivery token.
    pub const fn token(&self) -> &ResourceDeliveryClaimToken {
        &self.token
    }

    /// Return the heartbeat TTL.
    pub const fn ttl(&self) -> ResourceLeaseTtl {
        self.ttl
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Request to release an exact delivery token idempotently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseResourceDeliveryRequest {
    scope: Scope,
    delivery_id: ResourceDeliveryId,
    token: ResourceDeliveryClaimToken,
}

impl ReleaseResourceDeliveryRequest {
    /// Construct an exact-token delivery release request.
    pub const fn new(
        scope: Scope,
        delivery_id: ResourceDeliveryId,
        token: ResourceDeliveryClaimToken,
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

    /// Return the delivery identifier.
    pub const fn delivery_id(&self) -> ResourceDeliveryId {
        self.delivery_id
    }

    /// Return the exact delivery token.
    pub const fn token(&self) -> &ResourceDeliveryClaimToken {
        &self.token
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}
