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

/// Storage-minted identity carried to the provider for one prepared operation.
///
/// The same slot prepared again with the same fingerprint reuses this value, so
/// a destination that keys on it sees one operation across every retry and
/// recovery. Retaining it is not permission to invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId([u8; 16]);

impl OperationId {
    /// Reconstruct an operation identity from its durable bytes.
    ///
    /// Restricted to the storage layer for the same reason as
    /// [`EffectSlotId::from_storage_bytes`].
    #[must_use]
    pub const fn from_storage_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Durable bytes of this operation identity.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for OperationId {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// The originating attempt an effect slot is bound to.
///
/// Engine retries mint new attempts, so binding the generation is what stops a
/// stale worker's recovery from being mistaken for the current attempt's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttemptGeneration(u64);

impl AttemptGeneration {
    /// Wrap a monotone attempt counter.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationRecord {
    operation: PreparedOperation,
    fingerprint: RequestFingerprint,
    state: OperationState,
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
        }
    }

    /// The prepared operation this record describes.
    #[must_use]
    pub const fn operation(self) -> PreparedOperation {
        self.operation
    }

    /// Fingerprint the slot is bound to.
    #[must_use]
    pub const fn fingerprint(self) -> RequestFingerprint {
        self.fingerprint
    }

    /// Current durable state.
    #[must_use]
    pub const fn state(self) -> OperationState {
        self.state
    }
}

/// A known provider answer being committed against a prepared operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

    /// The caller's attempt generation is behind the durable binding.
    ///
    /// A superseded worker cannot commit an outcome for the current attempt.
    #[error("attempt generation is stale for this effect slot")]
    StaleFence {
        /// Slot whose fence rejected the write.
        slot_id: EffectSlotId,
        /// Generation currently bound to the slot.
        current: AttemptGeneration,
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
