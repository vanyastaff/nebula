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

/// Longest occurrence label the ledger admits, in bytes.
pub const MAX_OCCURRENCE_LABEL_BYTES: usize = 512;

/// Bounded reason an occurrence label was rejected before persistence.
///
/// An occurrence label is part of a slot's natural key, so it must be
/// reproducible byte-for-byte by a restarted owner and safe to render in
/// diagnostics. The admitted alphabet is therefore *visible ASCII*: every byte
/// in `0x21..=0x7E` (`u8::is_ascii_graphic`). Space, control characters, and
/// non-ASCII bytes are rejected, so two labels that render identically are
/// always the same label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum OccurrenceLabelViolation {
    /// The label is empty.
    #[error("occurrence label is empty")]
    Empty,
    /// The label exceeds [`MAX_OCCURRENCE_LABEL_BYTES`].
    #[error("occurrence label exceeds 512 bytes")]
    TooLong,
    /// The label contains a byte outside visible ASCII (`0x21..=0x7E`).
    #[error("occurrence label contains a byte outside visible ASCII")]
    InvalidByte,
}

/// Scoped natural address reconstructible before a prepare acknowledgement.
///
/// One node may own many slots: each intended effect gets its own occurrence
/// label (for example `node-effect/v1`, or `unit/v1/<resource>/<contract>/#3`
/// for the third unit a node submits against one contract). The label is the
/// only thing that separates two effects with identical request bytes, so it
/// must be derived deterministically — a resumed owner has to rebuild the same
/// label to find the slot it already prepared.
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

    /// Check that `label` is an admissible occurrence label.
    ///
    /// Admissible means 1..=[`MAX_OCCURRENCE_LABEL_BYTES`] bytes, every byte
    /// visible ASCII (`0x21..=0x7E`); see [`OccurrenceLabelViolation`].
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_storage_port::{EffectOccurrenceKey, OccurrenceLabelViolation};
    ///
    /// assert_eq!(EffectOccurrenceKey::validate_label("node-effect/v1"), Ok(()));
    /// assert_eq!(
    ///     EffectOccurrenceKey::validate_label("unit/v1/db.main/orders.insert/#3"),
    ///     Ok(())
    /// );
    /// assert_eq!(
    ///     EffectOccurrenceKey::validate_label("two words"),
    ///     Err(OccurrenceLabelViolation::InvalidByte)
    /// );
    /// ```
    pub const fn validate_label(label: &str) -> Result<(), OccurrenceLabelViolation> {
        let bytes = label.as_bytes();
        if bytes.is_empty() {
            return Err(OccurrenceLabelViolation::Empty);
        }
        if bytes.len() > MAX_OCCURRENCE_LABEL_BYTES {
            return Err(OccurrenceLabelViolation::TooLong);
        }
        let mut index = 0;
        while index < bytes.len() {
            if !bytes[index].is_ascii_graphic() {
                return Err(OccurrenceLabelViolation::InvalidByte);
            }
            index += 1;
        }
        Ok(())
    }

    /// Check this address's occurrence label before any durable access.
    ///
    /// Every adapter applies this to reads and preparations alike, so an
    /// inadmissible label can neither be stored nor probed.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::InvalidOccurrence`] naming the violated
    /// rule.
    pub const fn validate(self) -> Result<(), OperationLedgerError> {
        match Self::validate_label(self.occurrence) {
            Ok(()) => Ok(()),
            Err(violation) => Err(OperationLedgerError::InvalidOccurrence { violation }),
        }
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
    /// with identical payloads, and this is what tells them apart. Must pass
    /// [`EffectOccurrenceKey::validate_label`]; every adapter rejects an
    /// inadmissible label with [`OperationLedgerError::InvalidOccurrence`]
    /// before any durable access.
    pub occurrence: &'a str,
    /// Attempt generation that originated this slot.
    pub attempt_generation: AttemptGeneration,
    /// Fingerprint of the canonicalized request.
    pub fingerprint: RequestFingerprint,
    /// Guarantee the destination offered when this slot was prepared.
    pub destination: DestinationCapability,
    /// Complete concrete destination/descriptor binding and finite pinned policy.
    pub contract: &'a super::PreparedEffectContract,
    /// Idempotency key the provider receives for this slot, when the caller
    /// derives one.
    ///
    /// Persisted in the same transaction as the preparation and part of the
    /// prepare identity: re-preparing the occurrence with a different key, or
    /// with a key where none was recorded (or the reverse), is
    /// [`OperationLedgerError::OperationMismatch`] with no durable change. It
    /// is never part of the natural key. A resumed owner reads the recorded
    /// key back from [`PreparedOperation::provider_key`] instead of
    /// recomputing it.
    pub provider_key: Option<ProviderIdempotencyKey>,
    /// The lower positions of this occurrence's positional run whose unit
    /// was still open when this one was first prepared: they ran
    /// concurrently with it. Every other lower position had finished before
    /// it began. Canonical runs ([`PositionRange`](super::PositionRange):
    /// ascending, disjoint, not adjacent), at most
    /// [`OperationProtocolRecord::MAX_CONCURRENT_RANGES`](super::OperationProtocolRecord::MAX_CONCURRENT_RANGES):
    /// the exact set, never truncated — a longer list is an invalid
    /// protocol. `Some(&[])` when none was; `None` when the owner records no
    /// concurrency — the record's concurrency is then unknown.
    ///
    /// Persisted with the first preparation only, read back from
    /// [`OperationProtocolRecord::concurrent_with`](super::OperationProtocolRecord::concurrent_with).
    /// Never part of the natural key or of the prepare identity: re-preparing
    /// with another list replays the recorded slot unchanged.
    pub concurrent_with: Option<&'a [super::PositionRange]>,
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

/// Longest provider idempotency key the ledger admits, in bytes.
pub const MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES: usize = 64;

/// Durable, opaque idempotency key a provider receives for one effect slot.
///
/// The key is what lets a *provider* deduplicate a repeated call, so it must be
/// the same on every attempt of one slot: **a retry must reuse the key**. It
/// therefore never contains an attempt or retry number, and it is recorded in
/// the ledger at preparation — before any provider call — so a resumed owner
/// reads it back instead of recomputing it.
///
/// It is secret-free by construction: 1..=[`MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES`]
/// bytes of the base64url alphabet (`A-Z a-z 0-9 - _`, no padding). Callers
/// derive it as a digest (for example base64url of a SHA-256, 43 characters),
/// never from raw credentials or request payloads, which is why it may appear
/// in `Debug` output and diagnostics.
///
/// The value is stored inline, so the type stays `Copy`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProviderIdempotencyKey {
    len: u8,
    bytes: [u8; MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES],
}

/// Bounded reason a provider idempotency key was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum ProviderIdempotencyKeyError {
    /// The key is empty.
    #[error("provider idempotency key is empty")]
    Empty,
    /// The key exceeds [`MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES`].
    #[error("provider idempotency key exceeds 64 bytes")]
    TooLong,
    /// The key contains a character outside the base64url alphabet.
    #[error("provider idempotency key contains a character outside base64url")]
    InvalidCharacter,
}

impl ProviderIdempotencyKey {
    /// Validate and copy a key.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule: empty, longer than
    /// [`MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES`], or a character outside
    /// `A-Z a-z 0-9 - _`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_storage_port::{ProviderIdempotencyKey, ProviderIdempotencyKeyError};
    ///
    /// let key = ProviderIdempotencyKey::new("47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU")?;
    /// assert_eq!(key.as_str().len(), 43);
    /// assert_eq!(
    ///     ProviderIdempotencyKey::new("with=padding"),
    ///     Err(ProviderIdempotencyKeyError::InvalidCharacter)
    /// );
    /// # Ok::<(), ProviderIdempotencyKeyError>(())
    /// ```
    pub fn new(key: &str) -> Result<Self, ProviderIdempotencyKeyError> {
        let source = key.as_bytes();
        if source.is_empty() {
            return Err(ProviderIdempotencyKeyError::Empty);
        }
        let len = u8::try_from(source.len())
            .ok()
            .filter(|len| usize::from(*len) <= MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES)
            .ok_or(ProviderIdempotencyKeyError::TooLong)?;
        if !source
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ProviderIdempotencyKeyError::InvalidCharacter);
        }
        let mut bytes = [0; MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES];
        bytes
            .get_mut(..source.len())
            .ok_or(ProviderIdempotencyKeyError::TooLong)?
            .copy_from_slice(source);
        Ok(Self { len, bytes })
    }

    /// The key exactly as the provider receives it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.bytes
            .get(..usize::from(self.len))
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
            .unwrap_or_default()
    }
}

impl fmt::Debug for ProviderIdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProviderIdempotencyKey")
            .field(&self.as_str())
            .finish()
    }
}

impl fmt::Display for ProviderIdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl serde::Serialize for ProviderIdempotencyKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ProviderIdempotencyKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Self::new(&key).map_err(serde::de::Error::custom)
    }
}

/// One durably prepared operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedOperation {
    slot_id: EffectSlotId,
    operation_id: OperationId,
    attempt_generation: AttemptGeneration,
    destination: DestinationCapability,
    provider_key: Option<ProviderIdempotencyKey>,
}

impl PreparedOperation {
    /// Build a prepared-operation projection from durable state, without a
    /// provider idempotency key; see [`Self::with_provider_key`].
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
            provider_key: None,
        }
    }

    /// Attach the provider idempotency key recorded at preparation.
    #[must_use]
    pub const fn with_provider_key(mut self, provider_key: Option<ProviderIdempotencyKey>) -> Self {
        self.provider_key = provider_key;
        self
    }

    /// Provider idempotency key recorded at preparation, if any.
    ///
    /// Always the durable value: a replayed preparation returns the key the
    /// slot was first prepared with.
    #[must_use]
    pub const fn provider_key(self) -> Option<ProviderIdempotencyKey> {
        self.provider_key
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
    ///
    /// The protocol is the durable home of the provider idempotency key, so
    /// the attached protocol's key also becomes [`PreparedOperation::provider_key`]
    /// of [`Self::operation`].
    #[must_use]
    pub fn with_protocol(mut self, protocol: super::OperationProtocolRecord) -> Self {
        self.operation = self.operation.with_provider_key(protocol.provider_key());
        self.protocol = Some(protocol);
        self
    }
    /// Complete protocol state; legacy rows remain `None` and cannot invoke.
    pub const fn protocol(&self) -> Option<&super::OperationProtocolRecord> {
        self.protocol.as_ref()
    }
}

/// One slot of a node, as listed by
/// [`OperationLedger::read_occurrences`](crate::store::OperationLedger::read_occurrences).
///
/// Carries the occurrence label beside the record because the record alone
/// does not say which intended effect of the node it belongs to — and an owner
/// draining a node must match every durable slot to the effect it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectOccurrenceRecord {
    occurrence: String,
    record: OperationRecord,
}

impl EffectOccurrenceRecord {
    /// Pair a durable record with the occurrence label of its natural key.
    #[must_use]
    pub const fn new(occurrence: String, record: OperationRecord) -> Self {
        Self { occurrence, record }
    }

    /// Occurrence label of the slot's natural key.
    #[must_use]
    pub fn occurrence(&self) -> &str {
        &self.occurrence
    }

    /// Durable record of the slot.
    #[must_use]
    pub const fn record(&self) -> &OperationRecord {
        &self.record
    }

    /// Take the durable record, dropping the label.
    #[must_use]
    pub fn into_record(self) -> OperationRecord {
        self.record
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
    /// Recovery count or backend-clock deadline has been exhausted: a
    /// read-only query beyond its budget or window, or an invocation grant
    /// beyond the slot's total grant ceiling. Nothing was authorized and no
    /// state changed, so the refused call is known not sent.
    #[error("operation recovery budget is exhausted")]
    RecoveryExhausted,
    /// Attempt provenance exceeds the portable durable integer range.
    #[error("attempt generation is outside the supported durable range")]
    InvalidAttemptGeneration,
    /// The occurrence label is not admissible; nothing was read or written.
    #[error("invalid effect occurrence label: {violation}")]
    InvalidOccurrence {
        /// Bounded reason the label was rejected.
        violation: OccurrenceLabelViolation,
    },
    /// The execution is absent, outside this scope, or lacks this live lease.
    #[error("execution lease does not authorize this operation")]
    ExecutionLeaseRejected,
    /// The slot is bound to a different canonical request, contract, or
    /// provider idempotency key.
    ///
    /// Nothing was written. Reusing a slot for a different request would give
    /// two distinct effects one operation identity, and silently switching the
    /// provider key would let the provider apply one effect twice, so this
    /// fails closed.
    #[error("effect slot is bound to a different request or provider key")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrence_labels_admit_exactly_visible_ascii_within_the_cap() {
        for admitted in [
            "node-effect/v1",
            "unit/v1/db.main/orders.insert/#3",
            "!",
            "~",
        ] {
            assert_eq!(EffectOccurrenceKey::validate_label(admitted), Ok(()));
        }
        assert_eq!(
            EffectOccurrenceKey::validate_label(&"a".repeat(MAX_OCCURRENCE_LABEL_BYTES)),
            Ok(())
        );
        assert_eq!(
            EffectOccurrenceKey::validate_label(""),
            Err(OccurrenceLabelViolation::Empty)
        );
        assert_eq!(
            EffectOccurrenceKey::validate_label(&"a".repeat(MAX_OCCURRENCE_LABEL_BYTES + 1)),
            Err(OccurrenceLabelViolation::TooLong)
        );
        for rejected in ["two words", "tab\there", "nul\0", "del\u{7f}", "ünit"] {
            assert_eq!(
                EffectOccurrenceKey::validate_label(rejected),
                Err(OccurrenceLabelViolation::InvalidByte),
                "{rejected:?} must be rejected"
            );
        }
        let scope = Scope::new("ws", "org");
        assert_eq!(
            EffectOccurrenceKey::new(&scope, "exe", "node", "line\nbreak").validate(),
            Err(OperationLedgerError::InvalidOccurrence {
                violation: OccurrenceLabelViolation::InvalidByte
            })
        );
    }

    #[test]
    fn provider_keys_admit_only_bounded_base64url_and_round_trip() {
        let digest = "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU";
        let key = ProviderIdempotencyKey::new(digest).unwrap();
        assert_eq!(key.as_str(), digest);
        assert_eq!(key.to_string(), digest);
        assert!(format!("{key:?}").contains(digest));
        let longest = "A".repeat(MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES);
        assert_eq!(
            ProviderIdempotencyKey::new(&longest).unwrap().as_str(),
            longest
        );
        assert_eq!(
            ProviderIdempotencyKey::new(""),
            Err(ProviderIdempotencyKeyError::Empty)
        );
        assert_eq!(
            ProviderIdempotencyKey::new(&"A".repeat(MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES + 1)),
            Err(ProviderIdempotencyKeyError::TooLong)
        );
        for rejected in ["pad=", "plus+", "slash/", "space ", "ключ"] {
            assert_eq!(
                ProviderIdempotencyKey::new(rejected),
                Err(ProviderIdempotencyKeyError::InvalidCharacter),
                "{rejected:?} must be rejected"
            );
        }

        let encoded = serde_json::to_string(&key).unwrap();
        assert_eq!(encoded, format!("\"{digest}\""));
        assert_eq!(
            serde_json::from_str::<ProviderIdempotencyKey>(&encoded).unwrap(),
            key
        );
        assert!(serde_json::from_str::<ProviderIdempotencyKey>("\"no=pad\"").is_err());
    }

    #[test]
    fn a_keyless_protocol_record_serializes_as_before_and_a_key_round_trips() {
        let policy = super::super::PreparedEffectPolicy::builder(DestinationCapability::Opaque)
            .maximum_invocations(1)
            .maximum_queries(0)
            .recovery_window(std::time::Duration::from_secs(1))
            .build()
            .unwrap();
        let contract =
            super::super::PreparedEffectContract::new(RequestFingerprint::new(1, [7; 32]), policy)
                .unwrap();
        let keyless = super::super::OperationProtocolRecord::prepared(contract.clone(), 0)
            .build()
            .unwrap();
        let encoded = serde_json::to_value(&keyless).unwrap();
        assert!(
            encoded.get("provider_key").is_none(),
            "a keyless record must keep its pre-key bytes"
        );
        assert_eq!(
            serde_json::from_value::<super::super::OperationProtocolRecord>(encoded).unwrap(),
            keyless
        );

        let key = ProviderIdempotencyKey::new("order-123").unwrap();
        let keyed = super::super::OperationProtocolRecord::prepared(contract, 0)
            .provider_key(Some(key))
            .build()
            .unwrap();
        let decoded: super::super::OperationProtocolRecord =
            serde_json::from_value(serde_json::to_value(&keyed).unwrap()).unwrap();
        assert_eq!(decoded.provider_key(), Some(key));
        let record = OperationRecord::new(
            PreparedOperation::new(
                EffectSlotId::from_storage_bytes([1; 16]),
                OperationId::from_bytes([2; 16]),
                AttemptGeneration::new(0),
                DestinationCapability::Opaque,
            ),
            RequestFingerprint::new(1, [3; 32]),
            OperationState::Prepared,
        )
        .with_protocol(decoded);
        assert_eq!(record.operation().provider_key(), Some(key));
    }
}
