//! Durable operation-ledger values for the remote-effect protocol.
//!
//! A remote effect is not atomic with Nebula's database transaction, so the
//! ledger is what makes a repeated invocation *decidable* rather than
//! exactly-once. Runtime control durably prepares one operation before the
//! provider is called; every retry and recovery of that same effect slot then
//! carries the same [`OperationId`], and the persisted state records whether
//! the outcome is known, unknown, or merely unacknowledged.
//!
//! Two distinctions carry most of the weight:
//!
//! - **A slot is an intended occurrence, not a payload.** Two slots stay
//!   distinct even when their request bytes are identical, because "charge this
//!   card twice" is a legitimate program. Deduplication is per slot.
//! - **`OutcomeUnknown` is not a failure and `AcknowledgementUnknown` is not an
//!   outcome.** The first says the provider's answer was never learned; the
//!   second says our own database never confirmed the write. Collapsing either
//!   into "error" is what authorizes a duplicate effect.
//!
//! Holding an [`OperationId`] grants no invocation authority. Authority comes
//! from the destination's capability and the operation's current state, which
//! is why both are persisted alongside it.

use core::fmt;

use nebula_core::OperationId;

use crate::scope::Scope;

/// Storage-minted identity of one intended remote-effect occurrence.
///
/// Minted by the ledger, never by an adapter or an API surface: a caller that
/// could manufacture a slot identity could also silently merge two intended
/// occurrences into one, or split one into two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectSlotId([u8; 16]);

impl EffectSlotId {
    /// Reconstruct a slot identity from its durable bytes.
    ///
    /// Restricted to the storage layer: minting is the ledger's own authority,
    /// and this exists so an adapter can read a row back, not so a caller can
    /// invent one.
    #[must_use]
    pub const fn from_storage_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Durable bytes of this slot identity.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for EffectSlotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Digest of one canonicalized effect request.
///
/// Carries the canonicalization version because digests produced under
/// different rules are not comparable: two requests are "the same" only when
/// the same rules produced the same bytes. A version change therefore reads as
/// a mismatch rather than risking a false match that would reuse an operation
/// identity for a different request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RequestFingerprint {
    version: u16,
    digest: [u8; 32],
}

impl RequestFingerprint {
    /// Build a fingerprint from a digest and the rules that produced it.
    #[must_use]
    pub const fn new(version: u16, digest: [u8; 32]) -> Self {
        Self { version, digest }
    }

    /// Canonicalization version.
    #[must_use]
    pub const fn version(self) -> u16 {
        self.version
    }

    /// Raw digest bytes.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// What a destination allows after an ambiguous effect boundary.
///
/// This is the only thing that distinguishes a safe bounded retry from a
/// duplicate effect, so it is persisted with the operation rather than
/// re-derived at recovery time — a destination's configuration can change
/// between the prepare and the recovery, and the guarantee that applied is the
/// one recorded when the operation was prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum DestinationCapability {
    /// The destination honours a pinned stable key, so the *same* effecting
    /// call may be re-invoked as a bounded recovery of the same prepared
    /// operation while that guarantee holds.
    StableKey,
    /// The destination offers an authenticated read-only query, so an
    /// ambiguous outcome may be *reconciled* but the effecting call is never
    /// repeated.
    Reconcilable,
    /// Neither guarantee. An ambiguous boundary is terminal: the operation
    /// records `OutcomeUnknown` and only privileged adjudication can resolve it.
    Opaque,
}

/// Durable destination vocabulary rejected an unknown value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown durable destination capability")]
pub struct DestinationCapabilityParseError;

impl From<DestinationCapability> for &'static str {
    fn from(capability: DestinationCapability) -> Self {
        match capability {
            DestinationCapability::StableKey => "stable_key",
            DestinationCapability::Reconcilable => "reconcilable",
            DestinationCapability::Opaque => "opaque",
        }
    }
}

impl TryFrom<&str> for DestinationCapability {
    type Error = DestinationCapabilityParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "stable_key" => Ok(Self::StableKey),
            "reconcilable" => Ok(Self::Reconcilable),
            "opaque" => Ok(Self::Opaque),
            _ => Err(DestinationCapabilityParseError),
        }
    }
}

/// The originating attempt an effect slot is bound to.
///
/// Records originating attempt provenance. It grants no write authority;
/// adapters validate the execution's actual live fencing token separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttemptGeneration(u64);

impl AttemptGeneration {
    /// Wrap a monotone attempt counter. Durable preparation supports counters
    /// through `i64::MAX`; larger values are rejected without truncation.
    #[must_use]
    pub const fn new(generation: u64) -> Self {
        Self(generation)
    }

    /// Underlying counter.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Scoped natural address reconstructible before a prepare acknowledgement.
#[derive(Clone, Copy)]
pub struct EffectOccurrenceKey<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    node_key: &'a str,
    occurrence: &'a str,
}

impl<'a> EffectOccurrenceKey<'a> {
    /// Build a read address; these values grant no execution authority.
    #[must_use]
    pub const fn new(
        scope: &'a Scope,
        execution_id: &'a str,
        node_key: &'a str,
        occurrence: &'a str,
    ) -> Self {
        Self {
            scope,
            execution_id,
            node_key,
            occurrence,
        }
    }
    /// Tenant scope of the occurrence.
    #[must_use]
    pub const fn scope(self) -> &'a Scope {
        self.scope
    }
    /// Owning execution identity.
    #[must_use]
    pub const fn execution_id(self) -> &'a str {
        self.execution_id
    }
    /// Node issuing the occurrence.
    #[must_use]
    pub const fn node_key(self) -> &'a str {
        self.node_key
    }
    /// Label distinguishing effects within the node.
    #[must_use]
    pub const fn occurrence(self) -> &'a str {
        self.occurrence
    }
}

impl fmt::Debug for EffectOccurrenceKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectOccurrenceKey")
            .finish_non_exhaustive()
    }
}

/// Everything one durable preparation binds, before any provider is called.
#[derive(Debug, Clone)]
pub struct EffectSlotBinding<'a> {
    /// Tenant that owns the slot. One tenant can neither observe nor mutate
    /// another's operations.
    pub scope: &'a Scope,
    /// Execution the effect belongs to.
    pub execution_id: &'a str,
    /// Node within the execution that issues the effect.
    pub node_key: &'a str,
    /// Caller-chosen label distinguishing multiple effects from one node.
    ///
    /// Two intended occurrences from the same node are different slots even
    /// with identical payloads, and this is what tells them apart.
    pub occurrence: &'a str,
    /// Attempt generation that originated this slot.
    pub attempt_generation: AttemptGeneration,
    /// Fingerprint of the canonicalized request.
    pub fingerprint: RequestFingerprint,
    /// Guarantee the destination offered when this slot was prepared.
    pub destination: DestinationCapability,
    /// Complete concrete destination/descriptor binding and finite pinned policy.
    pub contract: &'a super::PreparedEffectContract,
}

impl EffectSlotBinding<'_> {
    /// Natural address retained even when preparation acknowledgement is lost.
    #[must_use]
    pub const fn occurrence_key(&self) -> EffectOccurrenceKey<'_> {
        EffectOccurrenceKey::new(
            self.scope,
            self.execution_id,
            self.node_key,
            self.occurrence,
        )
    }
}

/// One durably prepared operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedOperation {
    slot_id: EffectSlotId,
    operation_id: OperationId,
    attempt_generation: AttemptGeneration,
    destination: DestinationCapability,
}

impl PreparedOperation {
    /// Build a prepared-operation projection from durable state.
    #[must_use]
    pub const fn new(
        slot_id: EffectSlotId,
        operation_id: OperationId,
        attempt_generation: AttemptGeneration,
        destination: DestinationCapability,
    ) -> Self {
        Self {
            slot_id,
            operation_id,
            attempt_generation,
            destination,
        }
    }

    /// Storage-minted slot identity.
    #[must_use]
    pub const fn slot_id(self) -> EffectSlotId {
        self.slot_id
    }

    /// Identity the provider receives for this operation.
    #[must_use]
    pub const fn operation_id(self) -> OperationId {
        self.operation_id
    }

    /// Attempt generation the slot was originally bound to.
    #[must_use]
    pub const fn attempt_generation(self) -> AttemptGeneration {
        self.attempt_generation
    }

    /// Guarantee recorded when this operation was prepared.
    #[must_use]
    pub const fn destination(self) -> DestinationCapability {
        self.destination
    }
}

/// Result of durably preparing one effect slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareOutcome {
    /// The slot was absent and is now durably prepared.
    Prepared(PreparedOperation),
    /// The slot already existed with the same fingerprint. This is the
    /// original binding, including the original operation identity.
    Replayed(PreparedOperation),
}

impl PrepareOutcome {
    /// The prepared operation, whichever way it was reached.
    #[must_use]
    pub const fn operation(self) -> PreparedOperation {
        match self {
            Self::Prepared(operation) | Self::Replayed(operation) => operation,
        }
    }
}

/// Whether the provider's answer for one operation is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OperationState {
    /// Durably prepared; the provider may or may not have been called.
    Prepared,
    /// The provider accepted the effect.
    Succeeded,
    /// The provider rejected the effect without applying it.
    Failed,
    /// The boundary was crossed ambiguously and no bounded recovery remains.
    ///
    /// From here even a stable-key destination cannot re-invoke the effect;
    /// only authenticated read-only reconciliation or privileged audited
    /// adjudication may establish a known outcome.
    OutcomeUnknown,
}

/// The full durable record of one effect slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRecord {
    operation: PreparedOperation,
    fingerprint: RequestFingerprint,
    state: OperationState,
    protocol: Option<super::OperationProtocolRecord>,
}

impl OperationRecord {
    /// Build a record projection from durable state.
    #[must_use]
    pub const fn new(
        operation: PreparedOperation,
        fingerprint: RequestFingerprint,
        state: OperationState,
    ) -> Self {
        Self {
            operation,
            fingerprint,
            state,
            protocol: None,
        }
    }

    /// The prepared operation this record describes.
    #[must_use]
    pub const fn operation(&self) -> PreparedOperation {
        self.operation
    }

    /// Fingerprint the slot is bound to.
    #[must_use]
    pub const fn fingerprint(&self) -> RequestFingerprint {
        self.fingerprint
    }

    /// Current durable state.
    #[must_use]
    pub const fn state(&self) -> OperationState {
        self.state
    }
    /// Attach a decoded backend protocol projection; this grants no authority.
    #[must_use]
    pub fn with_protocol(mut self, protocol: super::OperationProtocolRecord) -> Self {
        self.protocol = Some(protocol);
        self
    }
    /// Complete protocol state; legacy rows remain `None` and cannot invoke.
    pub const fn protocol(&self) -> Option<&super::OperationProtocolRecord> {
        self.protocol.as_ref()
    }
}

/// A known provider answer being committed against a prepared operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum KnownOutcome {
    /// The provider accepted the effect.
    Succeeded,
    /// The provider rejected the effect without applying it.
    Failed,
    /// The boundary was crossed ambiguously and recovery is exhausted.
    OutcomeUnknown,
}

/// Closed, payload-redacted operation-ledger failure.
///
/// Variants carry only typed identifiers and bounded counters. Request
/// payloads, provider responses, credentials, SQL, and driver messages never
/// cross this boundary — an operation ledger sits directly beside the request
/// bodies it must never echo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OperationLedgerError {
    /// A caller supplied a structurally invalid protocol value.
    #[error("invalid operation protocol: {violation}")]
    ProtocolViolation {
        /// Bounded reason suitable for diagnostics.
        violation: OperationProtocolViolation,
    },
    /// Unsupported, malformed or unbounded protocol input.
    #[error("invalid operation protocol input")]
    InvalidProtocol,
    /// Protocol revision, outstanding call or disposition no longer matches.
    #[error("operation protocol transition is not permitted")]
    ProtocolConflict,
    /// Recovery count or backend-clock deadline has been exhausted.
    #[error("operation recovery budget is exhausted")]
    RecoveryExhausted,
    /// Attempt provenance exceeds the portable durable integer range.
    #[error("attempt generation is outside the supported durable range")]
    InvalidAttemptGeneration,
    /// The execution is absent, outside this scope, or lacks this live lease.
    #[error("execution lease does not authorize this operation")]
    ExecutionLeaseRejected,
    /// The slot is bound to a different canonical request.
    ///
    /// Nothing was written. Reusing a slot for a different request would give
    /// two distinct effects one operation identity, so this fails closed.
    #[error("effect slot is bound to a different request")]
    OperationMismatch {
        /// Slot whose binding differs.
        slot_id: EffectSlotId,
    },

    /// The named slot has no durable preparation.
    #[error("effect slot has no prepared operation")]
    SlotUnprepared {
        /// Slot that was read.
        slot_id: EffectSlotId,
    },

    /// The slot exists but belongs to another tenant.
    ///
    /// Deliberately indistinguishable from an absent slot to a caller that
    /// cannot see the tenant boundary: reporting "exists, but not yours" turns
    /// a guessed identity into a cross-tenant existence oracle.
    #[error("effect slot is not available in this tenant")]
    TenantDenied,

    /// A terminal outcome is already recorded and differs from this one.
    ///
    /// Outcomes are write-once. A second, different outcome would mean the
    /// ledger had recorded two answers for one effect.
    #[error("effect slot already recorded a different outcome")]
    OutcomeAlreadyRecorded {
        /// Slot whose outcome is already terminal.
        slot_id: EffectSlotId,
        /// Outcome the ledger holds.
        recorded: OperationState,
    },

    /// Durable state violates an invariant this build can interpret.
    #[error("operation ledger record is corrupt")]
    CorruptRecord {
        /// Slot whose record cannot be interpreted.
        slot_id: EffectSlotId,
    },

    /// The operation definitely did not commit.
    #[error("operation ledger is unavailable")]
    Unavailable,

    /// The commit was dispatched but its acknowledgement was lost.
    ///
    /// This is **not** a remote outcome. After a prepare, it authorizes zero
    /// provider calls until a database-only read confirms the exact durable
    /// binding; after an outcome commit, it authorizes only ledger reads and an
    /// exact recommit of the same evidence under the current fence.
    #[error("operation ledger acknowledgement is unknown; do not invoke the provider")]
    AcknowledgementUnknown,
}

/// Bounded reason a protocol value was rejected before persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OperationProtocolViolation {
    /// At least one provider invocation must be permitted.
    #[error("invocation limit must be between one and 10000")]
    InvocationLimit,
    /// Read-only reconciliation queries are bounded.
    #[error("query limit must not exceed 10000")]
    QueryLimit,
    /// Recovery must have a finite supported lifetime.
    #[error("recovery window must be between one millisecond and one year")]
    RecoveryWindow,
    /// Stable-key validity must be nonzero and fit inside recovery.
    #[error("stable-key window is inconsistent with the recovery window")]
    StableKeyWindow,
    /// Only stable-key destinations may carry a stable-key window.
    #[error("stable-key window does not match the destination capability")]
    StableKeyCapability,
    /// Opaque destinations cannot support an authoritative query.
    #[error("opaque destinations cannot permit reconciliation queries")]
    OpaqueQueries,
    /// The record schema version is unsupported.
    #[error("protocol record version is unsupported")]
    UnsupportedVersion,
    /// Counters exceed the pinned recovery policy.
    #[error("protocol counters exceed the pinned recovery policy")]
    CounterLimit,
    /// Phase, outstanding calls, and evidence are inconsistent.
    #[error("protocol phase and retained evidence are inconsistent")]
    InconsistentState,
    /// The backend timestamp cannot represent the complete recovery window.
    #[error("protocol recovery deadline is outside the supported time range")]
    RecoveryDeadline,
}
