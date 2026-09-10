//! Durable resource-event occurrence, envelope, and fanout values.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::shared_resource::framed_length;

use crate::Scope;

use super::{
    ResourceDeliveryClaimToken, ResourceLeaseGeneration, ResourceSourceLeaseToken,
    ResourceSubscriptionId, SharedResourceId,
};

const MAX_EVENT_NAMESPACE_BYTES: usize = 128;
const MAX_EVENT_OCCURRENCE_KEY_BYTES: usize = 1_024;
const MAX_EVENT_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Validation failure for event occurrence and envelope values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResourceEventValueError {
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

/// UTF-8 namespace for an upstream event occurrence key.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EventOccurrenceNamespace(String);

impl EventOccurrenceNamespace {
    /// Construct a namespace from `1..=128` UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceEventValueError`] when empty or too long.
    pub fn new(namespace: impl Into<String>) -> Result<Self, ResourceEventValueError> {
        let namespace = namespace.into();
        validate_required_bytes(
            "event occurrence namespace",
            namespace.as_bytes(),
            MAX_EVENT_NAMESPACE_BYTES,
        )?;
        Ok(Self(namespace))
    }

    /// Borrow the namespace.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EventOccurrenceNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventOccurrenceNamespace")
            .field("value", &self.0)
            .finish()
    }
}

/// Exact opaque key for one upstream occurrence within its namespace.
#[must_use]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EventOccurrenceKey(Box<[u8]>);

impl EventOccurrenceKey {
    /// Construct an occurrence key from `1..=1024` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceEventValueError`] when empty or too long.
    pub fn try_from_vec(bytes: Vec<u8>) -> Result<Self, ResourceEventValueError> {
        validate_required_bytes(
            "event occurrence key",
            &bytes,
            MAX_EVENT_OCCURRENCE_KEY_BYTES,
        )?;
        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Borrow the exact occurrence key bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for EventOccurrenceKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        redacted_len_debug(formatter, "EventOccurrenceKey", self.0.len())
    }
}

/// Canonical, versioned event envelope.
///
/// SHA-256 is only an index accelerator. Equality checks schema version and
/// every exact canonical payload byte after a digest match.
#[must_use]
#[derive(Clone)]
pub struct EventEnvelope {
    schema_version: u32,
    canonical_payload: Box<[u8]>,
    digest: [u8; 32],
}

impl EventEnvelope {
    /// Construct an envelope from `1..=1 MiB` canonical payload bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceEventValueError`] when the payload is empty or too long.
    pub fn try_from_vec(
        schema_version: u32,
        canonical_payload: Vec<u8>,
    ) -> Result<Self, ResourceEventValueError> {
        validate_required_bytes(
            "event envelope payload",
            &canonical_payload,
            MAX_EVENT_PAYLOAD_BYTES,
        )?;
        let digest = Sha256::digest(&canonical_payload).into();
        Ok(Self {
            schema_version,
            canonical_payload: canonical_payload.into_boxed_slice(),
            digest,
        })
    }

    /// Return the schema version participating in replay identity.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Borrow every exact canonical payload byte.
    pub fn canonical_payload(&self) -> &[u8] {
        &self.canonical_payload
    }

    /// Return the SHA-256 lookup accelerator.
    ///
    /// A matching digest is never proof that two envelopes are equal.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

impl PartialEq for EventEnvelope {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
            && self.schema_version == other.schema_version
            && self.canonical_payload == other.canonical_payload
    }
}

impl Eq for EventEnvelope {}

impl Hash for EventEnvelope {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.schema_version.hash(state);
        self.digest.hash(state);
    }
}

impl fmt::Debug for EventEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventEnvelope")
            .field("schema_version", &self.schema_version)
            .field("payload_len", &self.canonical_payload.len())
            .finish()
    }
}

/// Opaque store-minted resource-event identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceEventId([u8; 16]);

impl ResourceEventId {
    /// Rehydrate an identifier at a persistence boundary.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Return the exact persisted bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for ResourceEventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResourceEventId([opaque])")
    }
}

/// Opaque store-minted fanout-delivery identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceDeliveryId([u8; 16]);

impl ResourceDeliveryId {
    /// Rehydrate an identifier at a persistence boundary.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Return the exact persisted bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Debug for ResourceDeliveryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResourceDeliveryId([opaque])")
    }
}

/// Durable aggregate state for one accepted resource event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceEventState {
    /// At least one snapshotted delivery remains non-terminal.
    Pending,
    /// Every snapshotted delivery reached a terminal outcome.
    Complete,
}

impl ResourceEventState {
    /// Return the stable persistence vocabulary.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Complete => "complete",
        }
    }
}

impl FromStr for ResourceEventState {
    type Err = ResourceEventStateParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "complete" => Ok(Self::Complete),
            _ => Err(ResourceEventStateParseError),
        }
    }
}

/// Fail-closed persisted event-state parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("resource event state is unsupported")]
pub struct ResourceEventStateParseError;

/// Durable accepted resource event.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceEventRecord {
    id: ResourceEventId,
    resource_id: SharedResourceId,
    namespace: EventOccurrenceNamespace,
    occurrence_key: EventOccurrenceKey,
    envelope: EventEnvelope,
    accepted_at: DateTime<Utc>,
    source_generation: ResourceLeaseGeneration,
    state: ResourceEventState,
}

/// Authoritative acceptance time and source-generation provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEventAcceptance {
    accepted_at: DateTime<Utc>,
    source_generation: ResourceLeaseGeneration,
}

impl ResourceEventAcceptance {
    /// Rehydrate authoritative event-acceptance provenance.
    pub const fn new(
        accepted_at: DateTime<Utc>,
        source_generation: ResourceLeaseGeneration,
    ) -> Self {
        Self {
            accepted_at,
            source_generation,
        }
    }

    /// Return the authoritative backend acceptance timestamp.
    pub const fn accepted_at(self) -> DateTime<Utc> {
        self.accepted_at
    }

    /// Return the source generation that authorized first acceptance.
    pub const fn source_generation(self) -> ResourceLeaseGeneration {
        self.source_generation
    }
}

impl ResourceEventRecord {
    /// Rehydrate one durable event record.
    pub const fn new(
        id: ResourceEventId,
        resource_id: SharedResourceId,
        namespace: EventOccurrenceNamespace,
        occurrence_key: EventOccurrenceKey,
        envelope: EventEnvelope,
        acceptance: ResourceEventAcceptance,
        state: ResourceEventState,
    ) -> Self {
        Self {
            id,
            resource_id,
            namespace,
            occurrence_key,
            envelope,
            accepted_at: acceptance.accepted_at(),
            source_generation: acceptance.source_generation(),
            state,
        }
    }

    /// Return the store-minted event identifier.
    pub const fn id(&self) -> ResourceEventId {
        self.id
    }

    /// Return the originating resource.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the occurrence namespace.
    pub const fn namespace(&self) -> &EventOccurrenceNamespace {
        &self.namespace
    }

    /// Borrow the exact occurrence key.
    pub fn occurrence_key_bytes(&self) -> &[u8] {
        self.occurrence_key.as_bytes()
    }

    /// Return the stable digest accelerator for the exact occurrence identity.
    #[must_use]
    pub fn occurrence_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(framed_length(self.namespace.as_str().as_bytes()));
        hasher.update(self.namespace.as_str().as_bytes());
        hasher.update(framed_length(self.occurrence_key.as_bytes()));
        hasher.update(self.occurrence_key.as_bytes());
        hasher.finalize().into()
    }

    /// Return the exact envelope.
    pub const fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }

    /// Return the authoritative backend acceptance timestamp.
    pub const fn accepted_at(&self) -> DateTime<Utc> {
        self.accepted_at
    }

    /// Return the source generation that authorized first acceptance.
    pub const fn source_generation(&self) -> ResourceLeaseGeneration {
        self.source_generation
    }

    /// Return the event aggregate state.
    pub const fn state(&self) -> ResourceEventState {
        self.state
    }
}

impl fmt::Debug for ResourceEventRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceEventRecord")
            .field("id", &self.id)
            .field("resource_id", &self.resource_id)
            .field("namespace", &self.namespace)
            .field("occurrence_key_len", &self.occurrence_key.0.len())
            .field("envelope", &self.envelope)
            .field("accepted_at", &self.accepted_at)
            .field("source_generation", &self.source_generation)
            .field("state", &self.state)
            .finish()
    }
}

/// Request to atomically accept an event and snapshot Active subscriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptResourceEventRequest {
    scope: Scope,
    resource_id: SharedResourceId,
    source_token: ResourceSourceLeaseToken,
    namespace: EventOccurrenceNamespace,
    occurrence_key: EventOccurrenceKey,
    envelope: EventEnvelope,
}

impl AcceptResourceEventRequest {
    /// Construct an event acceptance request.
    pub const fn new(
        scope: Scope,
        resource_id: SharedResourceId,
        source_token: ResourceSourceLeaseToken,
        namespace: EventOccurrenceNamespace,
        occurrence_key: EventOccurrenceKey,
        envelope: EventEnvelope,
    ) -> Self {
        Self {
            scope,
            resource_id,
            source_token,
            namespace,
            occurrence_key,
            envelope,
        }
    }

    /// Return the requested scope.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Return the originating resource.
    pub const fn resource_id(&self) -> SharedResourceId {
        self.resource_id
    }

    /// Return the exact source authority proof.
    pub const fn source_token(&self) -> &ResourceSourceLeaseToken {
        &self.source_token
    }

    /// Return the occurrence namespace.
    pub const fn namespace(&self) -> &EventOccurrenceNamespace {
        &self.namespace
    }

    /// Borrow the exact occurrence key.
    pub fn occurrence_key_bytes(&self) -> &[u8] {
        self.occurrence_key.as_bytes()
    }

    /// Return the stable digest accelerator for the exact occurrence identity.
    #[must_use]
    pub fn occurrence_digest(&self) -> [u8; 32] {
        occurrence_digest(&self.namespace, &self.occurrence_key)
    }

    /// Return the exact event envelope.
    pub const fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

fn occurrence_digest(
    namespace: &EventOccurrenceNamespace,
    occurrence_key: &EventOccurrenceKey,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(framed_length(namespace.as_str().as_bytes()));
    hasher.update(namespace.as_str().as_bytes());
    hasher.update(framed_length(occurrence_key.as_bytes()));
    hasher.update(occurrence_key.as_bytes());
    hasher.finalize().into()
}

/// Typed result of accepting one occurrence identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptResourceEventOutcome {
    /// The event and one delivery per Active subscription were inserted atomically.
    Accepted {
        /// Store-minted event identifier.
        event_id: ResourceEventId,
        /// Number of Active subscriptions snapshotted into deliveries.
        delivery_count: u32,
    },
    /// The same occurrence already has the same schema and exact envelope bytes.
    Replayed {
        /// Existing event identifier.
        event_id: ResourceEventId,
    },
    /// The occurrence exists with a different schema or exact envelope bytes.
    Conflict {
        /// Existing event identifier. No supplied or persisted bytes are exposed.
        event_id: ResourceEventId,
    },
}

/// Closed terminal reason for a delivery that must never be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalDeliveryIneligibility {
    /// The snapshotted subscription was disabled before delivery.
    SubscriptionDisabled,
    /// The snapshotted subscription was tombstoned before delivery.
    SubscriptionTombstoned,
    /// The target consumer no longer exists.
    ConsumerUnavailable,
    /// The target consumer permanently rejects the envelope schema.
    UnsupportedEnvelopeSchema,
}

impl TerminalDeliveryIneligibility {
    /// Return the stable persistence vocabulary.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SubscriptionDisabled => "subscription_disabled",
            Self::SubscriptionTombstoned => "subscription_tombstoned",
            Self::ConsumerUnavailable => "consumer_unavailable",
            Self::UnsupportedEnvelopeSchema => "unsupported_envelope_schema",
        }
    }
}

impl FromStr for TerminalDeliveryIneligibility {
    type Err = TerminalDeliveryIneligibilityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "subscription_disabled" => Ok(Self::SubscriptionDisabled),
            "subscription_tombstoned" => Ok(Self::SubscriptionTombstoned),
            "consumer_unavailable" => Ok(Self::ConsumerUnavailable),
            "unsupported_envelope_schema" => Ok(Self::UnsupportedEnvelopeSchema),
            _ => Err(TerminalDeliveryIneligibilityParseError),
        }
    }
}

/// Fail-closed persisted terminal-ineligibility parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("resource delivery terminal ineligibility is unsupported")]
pub struct TerminalDeliveryIneligibilityParseError;

/// Terminal completion applied to one exact delivery claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceDeliveryCompletion {
    /// The workflow event was durably accepted by its target queue.
    Delivered,
    /// The delivery is permanently ineligible and must not be retried.
    Ineligible(TerminalDeliveryIneligibility),
}

/// One claimed fanout delivery with the exact event and consumer snapshot.
#[derive(Clone, PartialEq, Eq)]
pub struct ClaimedResourceDelivery {
    id: ResourceDeliveryId,
    event_id: ResourceEventId,
    subscription_id: ResourceSubscriptionId,
    envelope: EventEnvelope,
    token: ResourceDeliveryClaimToken,
}

impl ClaimedResourceDelivery {
    /// Rehydrate a claimed delivery.
    pub const fn new(
        id: ResourceDeliveryId,
        event_id: ResourceEventId,
        subscription_id: ResourceSubscriptionId,
        envelope: EventEnvelope,
        token: ResourceDeliveryClaimToken,
    ) -> Self {
        Self {
            id,
            event_id,
            subscription_id,
            envelope,
            token,
        }
    }

    /// Return the delivery identifier.
    pub const fn id(&self) -> ResourceDeliveryId {
        self.id
    }

    /// Return the parent event identifier.
    pub const fn event_id(&self) -> ResourceEventId {
        self.event_id
    }

    /// Return the snapshotted subscription identifier.
    pub const fn subscription_id(&self) -> ResourceSubscriptionId {
        self.subscription_id
    }

    /// Return the exact event envelope.
    pub const fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }

    /// Return the exact delivery claim proof.
    pub const fn token(&self) -> &ResourceDeliveryClaimToken {
        &self.token
    }
}

impl fmt::Debug for ClaimedResourceDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimedResourceDelivery")
            .field("id", &self.id)
            .field("event_id", &self.event_id)
            .field("subscription_id", &self.subscription_id)
            .field("envelope", &self.envelope)
            .field("token", &self.token)
            .finish()
    }
}

/// Request to complete one exact claimed delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteResourceDeliveryRequest {
    scope: Scope,
    delivery_id: ResourceDeliveryId,
    token: ResourceDeliveryClaimToken,
    completion: ResourceDeliveryCompletion,
}

impl CompleteResourceDeliveryRequest {
    /// Construct a delivery completion request.
    pub const fn new(
        scope: Scope,
        delivery_id: ResourceDeliveryId,
        token: ResourceDeliveryClaimToken,
        completion: ResourceDeliveryCompletion,
    ) -> Self {
        Self {
            scope,
            delivery_id,
            token,
            completion,
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

    /// Return the exact delivery claim proof.
    pub const fn token(&self) -> &ResourceDeliveryClaimToken {
        &self.token
    }

    /// Return the terminal completion.
    pub const fn completion(&self) -> ResourceDeliveryCompletion {
        self.completion
    }

    /// Replace the caller-supplied scope for a policy decorator.
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
}

/// Result of completing one exact delivery claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteResourceDeliveryOutcome {
    /// This call terminalized the delivery.
    Completed {
        /// Exact terminal completion persisted by this call.
        completion: ResourceDeliveryCompletion,
        /// Whether this completion also terminalized the parent event.
        event_became_terminal: bool,
    },
    /// The same terminal completion had already been recorded.
    AlreadyCompleted {
        /// Exact terminal completion already persisted.
        completion: ResourceDeliveryCompletion,
        /// Whether the parent event is terminal.
        event_is_terminal: bool,
    },
}

fn validate_required_bytes(
    field: &'static str,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<(), ResourceEventValueError> {
    if bytes.is_empty() {
        return Err(ResourceEventValueError::Empty { field });
    }
    if bytes.len() > max_bytes {
        return Err(ResourceEventValueError::TooLong { field, max_bytes });
    }
    Ok(())
}

fn redacted_len_debug(
    formatter: &mut fmt::Formatter<'_>,
    type_name: &'static str,
    len: usize,
) -> fmt::Result {
    formatter
        .debug_struct(type_name)
        .field("len", &len)
        .finish()
}
