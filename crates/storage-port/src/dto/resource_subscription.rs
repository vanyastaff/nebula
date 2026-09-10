//! Durable consumer subscriptions to shared runtime resources.

use std::fmt;
use std::str::FromStr;

use crate::Scope;

use super::{ReconciliationCursor, SharedResourceId};

const MAX_CONSUMER_KIND_BYTES: usize = 64;
const MAX_CONSUMER_IDENTITY_BYTES: usize = 512;

/// Validation failure for a subscription consumer value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResourceSubscriptionValueError {
    /// A required value was empty.
    #[error("{field} must not be empty")]
    Empty {
        /// Stable field name.
        field: &'static str,
    },
    /// A value exceeded its byte limit.
    #[error("{field} exceeds the {max_bytes}-byte limit")]
    TooLong {
        /// Stable field name.
        field: &'static str,
        /// Maximum accepted byte length.
        max_bytes: usize,
    },
}

/// Bounded UTF-8 class of a resource consumer.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceConsumerKind(String);

impl ResourceConsumerKind {
    /// Construct a consumer kind from `1..=64` UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceSubscriptionValueError`] when empty or too long.
    pub fn new(kind: impl Into<String>) -> Result<Self, ResourceSubscriptionValueError> {
        let kind = kind.into();
        validate_required_bytes(
            "resource consumer kind",
            kind.as_bytes(),
            MAX_CONSUMER_KIND_BYTES,
        )?;
        Ok(Self(kind))
    }

    /// Borrow the consumer kind.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ResourceConsumerKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceConsumerKind")
            .field("value", &self.0)
            .finish()
    }
}

/// Exact opaque identity of a consumer within its kind.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ResourceConsumerIdentity(Box<[u8]>);

impl ResourceConsumerIdentity {
    /// Construct an identity from `1..=512` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceSubscriptionValueError`] when empty or too long.
    pub fn try_from_vec(bytes: Vec<u8>) -> Result<Self, ResourceSubscriptionValueError> {
        validate_required_bytes(
            "resource consumer identity",
            &bytes,
            MAX_CONSUMER_IDENTITY_BYTES,
        )?;
        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Borrow the exact identity bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for ResourceConsumerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceConsumerIdentity")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Opaque store-minted subscription identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceSubscriptionId([u8; 16]);

impl ResourceSubscriptionId {
    /// Rehydrate an identifier at a persistence boundary.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Return the exact persisted bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for ResourceSubscriptionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResourceSubscriptionId([opaque])")
    }
}

/// Optimistic-CAS version for a resource subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceSubscriptionVersion(u64);

impl ResourceSubscriptionVersion {
    /// Construct a persisted CAS version.
    pub const fn new(version: u64) -> Self {
        Self(version)
    }

    /// Return the persisted version.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Durable subscription lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceSubscriptionState {
    /// The consumer participates in event fanout and keeps the resource live.
    Active,
    /// The consumer remains durable but is excluded from fanout.
    Disabled,
    /// The subscription is terminal and cannot be reactivated.
    Tombstoned,
}

impl ResourceSubscriptionState {
    /// Return the stable persistence vocabulary.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Tombstoned => "tombstoned",
        }
    }
}

impl fmt::Display for ResourceSubscriptionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ResourceSubscriptionState {
    type Err = ResourceSubscriptionStateParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            "tombstoned" => Ok(Self::Tombstoned),
            _ => Err(ResourceSubscriptionStateParseError),
        }
    }
}

/// Fail-closed persisted subscription-state parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("resource subscription state is unsupported")]
pub struct ResourceSubscriptionStateParseError;

/// Durable resource subscription record.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceSubscriptionRecord {
    id: ResourceSubscriptionId,
    resource_id: SharedResourceId,
    consumer_kind: ResourceConsumerKind,
    consumer_identity: ResourceConsumerIdentity,
    state: ResourceSubscriptionState,
    version: ResourceSubscriptionVersion,
    reconciliation_sequence: u64,
}

impl ResourceSubscriptionRecord {
    /// Rehydrate a durable subscription record.
    pub const fn new(
        id: ResourceSubscriptionId,
        resource_id: SharedResourceId,
        consumer_kind: ResourceConsumerKind,
        consumer_identity: ResourceConsumerIdentity,
        state: ResourceSubscriptionState,
        version: ResourceSubscriptionVersion,
        reconciliation_sequence: u64,
    ) -> Self {
        Self {
            id,
            resource_id,
            consumer_kind,
            consumer_identity,
            state,
            version,
            reconciliation_sequence,
        }
    }

    /// Return the store-minted subscription identifier.
    pub const fn id(&self) -> ResourceSubscriptionId {
        self.id
    }

    /// Return the subscribed resource identifier.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the consumer kind.
    pub const fn consumer_kind(&self) -> &ResourceConsumerKind {
        &self.consumer_kind
    }

    /// Return the exact consumer identity bytes.
    pub fn consumer_identity_bytes(&self) -> &[u8] {
        self.consumer_identity.as_bytes()
    }

    /// Return the durable lifecycle state.
    pub const fn state(&self) -> ResourceSubscriptionState {
        self.state
    }

    /// Return the optimistic-CAS version.
    pub const fn version(&self) -> ResourceSubscriptionVersion {
        self.version
    }

    /// Return the backend-authored sequence used by stable restart scans.
    pub const fn reconciliation_sequence(&self) -> u64 {
        self.reconciliation_sequence
    }
}

impl fmt::Debug for ResourceSubscriptionRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceSubscriptionRecord")
            .field("id", &self.id)
            .field("resource_id", &self.resource_id)
            .field("consumer_kind", &self.consumer_kind)
            .field("consumer_identity_len", &self.consumer_identity.0.len())
            .field("state", &self.state)
            .field("version", &self.version)
            .field("reconciliation_sequence", &self.reconciliation_sequence)
            .finish()
    }
}

/// One bounded subscription page ordered by backend sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSubscriptionPage {
    subscriptions: Vec<ResourceSubscriptionRecord>,
    next_cursor: Option<ReconciliationCursor>,
}

impl ResourceSubscriptionPage {
    /// Rehydrate a bounded page at a persistence boundary.
    pub fn new(
        subscriptions: Vec<ResourceSubscriptionRecord>,
        next_cursor: Option<ReconciliationCursor>,
    ) -> Self {
        Self {
            subscriptions,
            next_cursor,
        }
    }

    /// Borrow the ordered records.
    pub fn subscriptions(&self) -> &[ResourceSubscriptionRecord] {
        &self.subscriptions
    }

    /// Return the exclusive cursor for a following page.
    pub const fn next_cursor(&self) -> Option<ReconciliationCursor> {
        self.next_cursor
    }
}

/// Request to create or resolve one exact resource subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutResourceSubscriptionRequest {
    scope: Scope,
    resource_id: SharedResourceId,
    consumer_kind: ResourceConsumerKind,
    consumer_identity: ResourceConsumerIdentity,
}

impl PutResourceSubscriptionRequest {
    /// Construct a subscription request.
    pub const fn new(
        scope: Scope,
        resource_id: SharedResourceId,
        consumer_kind: ResourceConsumerKind,
        consumer_identity: ResourceConsumerIdentity,
    ) -> Self {
        Self {
            scope,
            resource_id,
            consumer_kind,
            consumer_identity,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the target resource.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the consumer kind.
    pub const fn consumer_kind(&self) -> &ResourceConsumerKind {
        &self.consumer_kind
    }

    /// Borrow the exact consumer identity.
    pub fn consumer_identity_bytes(&self) -> &[u8] {
        self.consumer_identity.as_bytes()
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Result of creating or resolving a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutResourceSubscriptionOutcome {
    /// A new Active subscription was inserted.
    Created(ResourceSubscriptionRecord),
    /// The exact subscription already existed.
    Existing(ResourceSubscriptionRecord),
}

/// CAS request for a subscription lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionResourceSubscriptionRequest {
    scope: Scope,
    subscription_id: ResourceSubscriptionId,
    expected_version: ResourceSubscriptionVersion,
    target_state: ResourceSubscriptionState,
}

impl TransitionResourceSubscriptionRequest {
    /// Construct a lifecycle transition request.
    pub const fn new(
        scope: Scope,
        subscription_id: ResourceSubscriptionId,
        expected_version: ResourceSubscriptionVersion,
        target_state: ResourceSubscriptionState,
    ) -> Self {
        Self {
            scope,
            subscription_id,
            expected_version,
            target_state,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the subscription identifier.
    pub const fn subscription_id(&self) -> ResourceSubscriptionId {
        self.subscription_id
    }

    /// Return the expected CAS version.
    pub const fn expected_version(&self) -> ResourceSubscriptionVersion {
        self.expected_version
    }

    /// Return the requested target state.
    pub const fn target_state(&self) -> ResourceSubscriptionState {
        self.target_state
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

fn validate_required_bytes(
    field: &'static str,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<(), ResourceSubscriptionValueError> {
    if bytes.is_empty() {
        return Err(ResourceSubscriptionValueError::Empty { field });
    }
    if bytes.len() > max_bytes {
        return Err(ResourceSubscriptionValueError::TooLong { field, max_bytes });
    }
    Ok(())
}
