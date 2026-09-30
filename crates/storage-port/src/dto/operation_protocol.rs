//! Versioned, payload-redacted records for bounded effect recovery.

use std::time::Duration;

use nebula_core::OperationCallId;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest as _, Sha256};

use super::{
    DestinationCapability, KnownOutcome, OperationLedgerError, OperationProtocolViolation,
    ProviderIdempotencyKey, RequestFingerprint,
};

const fn violation(violation: OperationProtocolViolation) -> OperationLedgerError {
    OperationLedgerError::ProtocolViolation { violation }
}

/// Static destination guarantee and finite recovery limits, pinned in the plan.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEffectPolicy {
    capability: DestinationCapability,
    max_invocations: u32,
    max_queries: u32,
    recovery_window_ms: u64,
    stable_window_ms: Option<u64>,
}

/// Named construction of the durable projection of an effect policy.
#[derive(Debug)]
#[must_use = "a prepared effect policy builder must be completed with `build`"]
pub struct PreparedEffectPolicyBuilder {
    capability: DestinationCapability,
    max_invocations: Option<u32>,
    max_queries: Option<u32>,
    recovery_window: Option<Duration>,
    stable_window: Option<Duration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedEffectPolicyWire {
    capability: DestinationCapability,
    max_invocations: u32,
    max_queries: u32,
    recovery_window_ms: u64,
    stable_window_ms: Option<u64>,
}

impl PreparedEffectPolicy {
    /// Begin a named declaration of finite durable recovery limits.
    pub const fn builder(capability: DestinationCapability) -> PreparedEffectPolicyBuilder {
        PreparedEffectPolicyBuilder {
            capability,
            max_invocations: None,
            max_queries: None,
            recovery_window: None,
            stable_window: None,
        }
    }

    fn from_milliseconds(
        capability: DestinationCapability,
        max_invocations: u32,
        max_queries: u32,
        recovery_window_ms: u64,
        stable_window_ms: Option<u64>,
    ) -> Result<Self, OperationLedgerError> {
        let contract = Self {
            capability,
            max_invocations,
            max_queries,
            recovery_window_ms,
            stable_window_ms,
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Validate decoded technical projections before any durable mutation.
    ///
    /// # Errors
    ///
    /// [`OperationLedgerError::ProtocolViolation`] when `max_invocations` is zero
    /// or above 10 000, `max_queries` is above 10 000, `recovery_window_ms` is
    /// zero or above one year, `stable_window_ms` is zero or wider than the
    /// recovery window, a stable window is present without
    /// [`DestinationCapability::StableKey`] (or absent with it), or an
    /// [`DestinationCapability::Opaque`] destination permits queries.
    pub fn validate(&self) -> Result<(), OperationLedgerError> {
        if self.max_invocations == 0 || self.max_invocations > 10_000 {
            return Err(violation(OperationProtocolViolation::InvocationLimit));
        }
        if self.max_queries > 10_000 {
            return Err(violation(OperationProtocolViolation::QueryLimit));
        }
        if self.recovery_window_ms == 0 || self.recovery_window_ms > 31_536_000_000 {
            return Err(violation(OperationProtocolViolation::RecoveryWindow));
        }
        if self
            .stable_window_ms
            .is_some_and(|window| window == 0 || window > self.recovery_window_ms)
        {
            return Err(violation(OperationProtocolViolation::StableKeyWindow));
        }
        if (self.capability == DestinationCapability::StableKey) != self.stable_window_ms.is_some()
        {
            return Err(violation(OperationProtocolViolation::StableKeyCapability));
        }
        if self.capability == DestinationCapability::Opaque && self.max_queries != 0 {
            return Err(violation(OperationProtocolViolation::OpaqueQueries));
        }
        Ok(())
    }
    /// Original destination capability.
    pub const fn capability(&self) -> DestinationCapability {
        self.capability
    }
    /// Total permitted effect calls, including the first call.
    pub const fn max_invocations(&self) -> u32 {
        self.max_invocations
    }
    /// Total permitted authenticated read-only queries.
    pub const fn max_queries(&self) -> u32 {
        self.max_queries
    }
    /// Recovery lifetime from preparation, in milliseconds.
    pub const fn recovery_window_ms(&self) -> u64 {
        self.recovery_window_ms
    }
    /// Stable-key lifetime from preparation, in milliseconds.
    pub const fn stable_window_ms(&self) -> Option<u64> {
        self.stable_window_ms
    }
}

impl PreparedEffectPolicyBuilder {
    /// Set the total permitted provider invocations, including the first call.
    pub const fn maximum_invocations(mut self, maximum: u32) -> Self {
        self.max_invocations = Some(maximum);
        self
    }

    /// Set the total permitted authenticated read-only queries.
    pub const fn maximum_queries(mut self, maximum: u32) -> Self {
        self.max_queries = Some(maximum);
        self
    }

    /// Set the finite lifetime available for recovery.
    pub const fn recovery_window(mut self, window: Duration) -> Self {
        self.recovery_window = Some(window);
        self
    }

    /// Set the provider's stable-key lifetime.
    pub const fn stable_key_window(mut self, window: Duration) -> Self {
        self.stable_window = Some(window);
        self
    }

    /// Validate and finish the durable policy projection.
    ///
    /// # Errors
    /// Returns a specific [`OperationProtocolViolation`] through
    /// [`OperationLedgerError::ProtocolViolation`] when a required limit is
    /// absent, a duration cannot be represented, or the complete policy is
    /// incoherent.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use nebula_storage_port::{DestinationCapability, PreparedEffectPolicy};
    ///
    /// let policy = PreparedEffectPolicy::builder(DestinationCapability::Opaque)
    ///     .maximum_invocations(1)
    ///     .maximum_queries(0)
    ///     .recovery_window(Duration::from_mins(1))
    ///     .build()?;
    /// assert_eq!(policy.max_queries(), 0);
    /// # Ok::<(), nebula_storage_port::OperationLedgerError>(())
    /// ```
    pub fn build(self) -> Result<PreparedEffectPolicy, OperationLedgerError> {
        let max_invocations = self
            .max_invocations
            .ok_or_else(|| violation(OperationProtocolViolation::InvocationLimit))?;
        let max_queries = self
            .max_queries
            .ok_or_else(|| violation(OperationProtocolViolation::QueryLimit))?;
        let recovery_window = self
            .recovery_window
            .ok_or_else(|| violation(OperationProtocolViolation::RecoveryWindow))?;
        let recovery_window_ms = u64::try_from(recovery_window.as_millis())
            .map_err(|_| violation(OperationProtocolViolation::RecoveryWindow))?;
        let stable_window_ms = self
            .stable_window
            .map(|window| {
                u64::try_from(window.as_millis())
                    .map_err(|_| violation(OperationProtocolViolation::StableKeyWindow))
            })
            .transpose()?;
        PreparedEffectPolicy::from_milliseconds(
            self.capability,
            max_invocations,
            max_queries,
            recovery_window_ms,
            stable_window_ms,
        )
    }
}

impl TryFrom<PreparedEffectPolicyWire> for PreparedEffectPolicy {
    type Error = OperationLedgerError;

    fn try_from(wire: PreparedEffectPolicyWire) -> Result<Self, Self::Error> {
        Self::from_milliseconds(
            wire.capability,
            wire.max_invocations,
            wire.max_queries,
            wire.recovery_window_ms,
            wire.stable_window_ms,
        )
    }
}

impl<'de> Deserialize<'de> for PreparedEffectPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(PreparedEffectPolicyWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Debug for PreparedEffectPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedEffectPolicy")
            .field("capability", &self.capability)
            .finish_non_exhaustive()
    }
}

/// Concrete non-secret destination/account/auth and static descriptor commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedEffectContract {
    identity: RequestFingerprint,
    policy: PreparedEffectPolicy,
}
impl PreparedEffectContract {
    /// Bind the actual prepared destination and exact descriptor to static policy.
    ///
    /// # Errors
    ///
    /// [`OperationLedgerError::ProtocolViolation`] when `policy` is not a
    /// coherent policy — see [`PreparedEffectPolicy::validate`].
    pub fn new(
        identity: RequestFingerprint,
        policy: PreparedEffectPolicy,
    ) -> Result<Self, OperationLedgerError> {
        policy.validate()?;
        Ok(Self { identity, policy })
    }
    /// Validate decoded policy before admitting a mutation.
    ///
    /// # Errors
    ///
    /// [`OperationLedgerError::ProtocolViolation`] when the decoded policy is not
    /// a coherent policy — see [`PreparedEffectPolicy::validate`].
    pub fn validate(&self) -> Result<(), OperationLedgerError> {
        self.policy.validate()
    }
    /// Complete destination and descriptor binding digest, with its version.
    pub const fn identity(&self) -> RequestFingerprint {
        self.identity
    }
    /// Static finite recovery policy, identical to the pinned action descriptor.
    pub const fn policy(&self) -> &PreparedEffectPolicy {
        &self.policy
    }
}

/// What the trusted invocation adapter proved about its effect boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum InvocationDisposition {
    /// No provider effect boundary was crossed.
    BeforeBoundary,
    /// The provider may have accepted the effect.
    Ambiguous,
}

/// Durable phase; an unexplained outstanding call never grants another call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EffectPhase {
    /// No invocation permit has been issued.
    Prepared,
    /// A permit was issued; its provider boundary is not durably explained.
    InvocationOutstanding,
    /// The latest call was proven not to cross the boundary.
    BeforeBoundary,
    /// The latest call crossed an ambiguous boundary.
    Ambiguous,
    /// No further effect invocation may be granted.
    OutcomeUnknown,
    /// A known immutable outcome is recorded.
    Resolved,
}

/// Source of the immutable evidence, bound to the exact outstanding call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum OutcomeEvidenceSource {
    /// Result returned by an effect invocation.
    Invocation(OperationCallId),
    /// Authoritative result of an authenticated read-only query.
    Reconciliation(OperationCallId),
    /// Privileged operator decision; ordinary runtime advance rejects this source.
    Adjudication(OperationCallId),
}

/// Exact serialized outcome, retained across database acknowledgement uncertainty.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct FrozenOutcomeEvidence {
    version: u16,
    source: OutcomeEvidenceSource,
    outcome: KnownOutcome,
    payload: Vec<u8>,
    digest: [u8; 32],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenOutcomeEvidenceWire {
    version: u16,
    source: OutcomeEvidenceSource,
    outcome: KnownOutcome,
    payload: Vec<u8>,
    digest: [u8; 32],
}

impl TryFrom<FrozenOutcomeEvidenceWire> for FrozenOutcomeEvidence {
    type Error = OperationLedgerError;

    fn try_from(wire: FrozenOutcomeEvidenceWire) -> Result<Self, Self::Error> {
        let evidence = Self {
            version: wire.version,
            source: wire.source,
            outcome: wire.outcome,
            payload: wire.payload,
            digest: wire.digest,
        };
        evidence.validate()?;
        Ok(evidence)
    }
}

impl<'de> Deserialize<'de> for FrozenOutcomeEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(FrozenOutcomeEvidenceWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

impl FrozenOutcomeEvidence {
    /// Freeze version-one JSON bytes, including the runtime's result disposition.
    /// Known effect with unavailable output must be represented explicitly in JSON.
    ///
    /// # Errors
    ///
    /// [`OperationLedgerError::InvalidProtocol`] when `payload` is empty, above
    /// 1 MiB, or not valid JSON, or when `outcome` is neither
    /// [`KnownOutcome::Succeeded`] nor [`KnownOutcome::Failed`].
    pub fn v1_json(
        source: OutcomeEvidenceSource,
        outcome: KnownOutcome,
        payload: Vec<u8>,
    ) -> Result<Self, OperationLedgerError> {
        let digest = Sha256::digest(&payload).into();
        let evidence = Self {
            version: 1,
            source,
            outcome,
            payload,
            digest,
        };
        evidence.validate()?;
        Ok(evidence)
    }
    /// Validate bounded syntax; runtime owns the ActionResult schema.
    ///
    /// # Errors
    ///
    /// Returns [`OperationLedgerError::InvalidProtocol`] for an unsupported
    /// version, empty or oversized payload, non-terminal outcome, digest
    /// mismatch, or invalid JSON.
    pub fn validate(&self) -> Result<(), OperationLedgerError> {
        if self.version != 1
            || self.payload.is_empty()
            || self.payload.len() > 1_048_576
            || !matches!(self.outcome, KnownOutcome::Succeeded | KnownOutcome::Failed)
        {
            return Err(OperationLedgerError::InvalidProtocol);
        }
        let digest: [u8; 32] = Sha256::digest(&self.payload).into();
        if digest != self.digest {
            return Err(OperationLedgerError::InvalidProtocol);
        }
        serde_json::from_slice::<serde_json::Value>(&self.payload)
            .map_err(|_| OperationLedgerError::InvalidProtocol)?;
        Ok(())
    }
    /// Evidence format version.
    pub const fn version(&self) -> u16 {
        self.version
    }
    /// Exact source invocation/query.
    pub const fn source(&self) -> OutcomeEvidenceSource {
        self.source
    }
    /// Known provider outcome.
    pub const fn outcome(&self) -> KnownOutcome {
        self.outcome
    }
    /// Original serialized bytes; never render in diagnostics.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    /// Integrity digest of the exact retained bytes.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}
impl std::fmt::Debug for FrozenOutcomeEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrozenOutcomeEvidence")
            .field("version", &self.version)
            .field("source", &self.source)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes the field by reference"
)]
const fn is_zero(count: &u32) -> bool {
    *count == 0
}

/// Read-only durable protocol projection. Deserializing never grants invocation authority.
///
/// # Budget accounting
///
/// `invocations` counts every issued permit; `not_crossed` counts the permits
/// whose call was durably proven not to cross the provider boundary
/// ([`InvocationDisposition::BeforeBoundary`]). Only the difference — the calls
/// that *may* have reached the provider — spends the pinned
/// [`PreparedEffectPolicy::max_invocations`] budget, and the recovery and
/// stable-key windows constrain a slot only once at least one call may have
/// crossed. Total permits stay bounded by [`Self::GRANT_CEILING`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationProtocolRecord {
    version: u16,
    contract: PreparedEffectContract,
    revision: u64,
    phase: EffectPhase,
    prepared_at_ms: i64,
    invocations: u32,
    /// Absent (not `0`) when no permit was proven not crossed, so such a
    /// record serializes byte-identically to one written before the counter
    /// existed.
    #[serde(default, skip_serializing_if = "is_zero")]
    not_crossed: u32,
    queries: u32,
    invocation: Option<OperationCallId>,
    disposition: Option<InvocationDisposition>,
    query: Option<OperationCallId>,
    evidence: Option<FrozenOutcomeEvidence>,
    adjudication_audit_digest: Option<[u8; 32]>,
    /// Absent (not `null`) when no key was recorded, so a keyless record
    /// serializes byte-identically to one written before keys existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_key: Option<ProviderIdempotencyKey>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationProtocolRecordWire {
    version: u16,
    contract: PreparedEffectContract,
    revision: u64,
    phase: EffectPhase,
    prepared_at_ms: i64,
    invocations: u32,
    /// Absent both for a zero counter and for a record written before the
    /// counter existed; see [`legacy_not_crossed`].
    #[serde(default)]
    not_crossed: Option<u32>,
    queries: u32,
    invocation: Option<OperationCallId>,
    disposition: Option<InvocationDisposition>,
    query: Option<OperationCallId>,
    evidence: Option<FrozenOutcomeEvidence>,
    adjudication_audit_digest: Option<[u8; 32]>,
    #[serde(default)]
    provider_key: Option<ProviderIdempotencyKey>,
}

/// The not-crossed count of a record that does not carry the counter.
///
/// A current record omits the counter only when it is zero, and a current
/// record whose latest call is `BeforeBoundary` always counts at least that
/// call — so an absent counter beside a `BeforeBoundary` disposition can only
/// be a record written before the counter existed. Its latest call was proven
/// not to cross, so counting exactly that one is truthful; earlier calls stay
/// counted as possibly crossed, which is the conservative reading.
fn legacy_not_crossed(disposition: Option<InvocationDisposition>) -> u32 {
    u32::from(disposition == Some(InvocationDisposition::BeforeBoundary))
}

impl OperationProtocolRecord {
    /// Total invocation permits one slot may ever be issued, crossed or not.
    ///
    /// Calls proven not to cross the provider boundary do not spend the
    /// policy budget, so this ceiling is what keeps a slot whose every call is
    /// refused locally from being granted forever.
    pub const GRANT_CEILING: u32 = 10_000;

    /// Begin a validated record with no issued permits.
    pub fn prepared(
        contract: PreparedEffectContract,
        prepared_at_ms: i64,
    ) -> OperationProtocolRecordBuilder {
        OperationProtocolRecordBuilder {
            record: Self {
                version: 1,
                contract,
                revision: 0,
                phase: EffectPhase::Prepared,
                prepared_at_ms,
                invocations: 0,
                not_crossed: 0,
                queries: 0,
                invocation: None,
                disposition: None,
                query: None,
                evidence: None,
                adjudication_audit_digest: None,
                provider_key: None,
            },
        }
    }

    /// Begin a named update of an existing valid record.
    pub fn rebuild(&self) -> OperationProtocolRecordBuilder {
        OperationProtocolRecordBuilder {
            record: self.clone(),
        }
    }

    /// Validate the complete protocol state.
    ///
    /// # Errors
    /// Returns a specific [`OperationProtocolViolation`] when any field is
    /// inconsistent with the pinned policy or durable phase:
    /// [`OperationProtocolViolation::CounterLimit`] when `not_crossed` exceeds
    /// `invocations`, `invocations` exceeds [`Self::GRANT_CEILING`], the
    /// possibly-crossed calls exceed the policy's invocation budget, or
    /// queries exceed theirs; [`OperationProtocolViolation::InconsistentState`]
    /// when the phase, disposition and counters disagree (for example a
    /// `Prepared` record with a not-crossed call, or a `BeforeBoundary`
    /// disposition without one).
    pub fn validate(&self) -> Result<(), OperationLedgerError> {
        self.contract.validate()?;
        if self.version != 1 {
            return Err(violation(OperationProtocolViolation::UnsupportedVersion));
        }
        if self.not_crossed > self.invocations
            || self.invocations > Self::GRANT_CEILING
            || self.crossed_invocations() > self.contract.policy().max_invocations()
            || self.queries > self.contract.policy().max_queries()
        {
            return Err(violation(OperationProtocolViolation::CounterLimit));
        }
        if self
            .prepared_at_ms
            .checked_add(
                i64::try_from(self.contract.policy().recovery_window_ms())
                    .map_err(|_| violation(OperationProtocolViolation::RecoveryDeadline))?,
            )
            .is_none()
        {
            return Err(violation(OperationProtocolViolation::RecoveryDeadline));
        }
        let counters_and_phase_are_consistent = (self.invocations == 0)
            == self.invocation.is_none()
            && (self.phase == EffectPhase::Resolved) == self.evidence.is_some()
            && self.revision >= u64::from(self.invocations) + u64::from(self.queries)
            && self.query.is_none_or(|_| {
                self.queries > 0
                    && self.invocations > 0
                    && matches!(
                        self.phase,
                        EffectPhase::OutcomeUnknown | EffectPhase::Resolved
                    )
            })
            // Recording a `BeforeBoundary` disposition always counts its call.
            && (self.disposition != Some(InvocationDisposition::BeforeBoundary)
                || self.not_crossed >= 1)
            && match self.phase {
                EffectPhase::Prepared => {
                    self.invocations == 0 && self.not_crossed == 0 && self.disposition.is_none()
                },
                // The outstanding call is unexplained, so it counts as crossed.
                EffectPhase::InvocationOutstanding => {
                    self.invocations > 0
                        && self.not_crossed < self.invocations
                        && self.disposition.is_none()
                },
                EffectPhase::BeforeBoundary => {
                    self.invocations > 0
                        && self.not_crossed >= 1
                        && self.disposition == Some(InvocationDisposition::BeforeBoundary)
                },
                EffectPhase::Ambiguous => {
                    self.invocations > 0
                        && self.not_crossed < self.invocations
                        && self.disposition == Some(InvocationDisposition::Ambiguous)
                        && self.contract.policy().capability() == DestinationCapability::StableKey
                },
                EffectPhase::OutcomeUnknown | EffectPhase::Resolved => true,
            };
        if !counters_and_phase_are_consistent {
            return Err(violation(OperationProtocolViolation::InconsistentState));
        }
        if let Some(evidence) = &self.evidence {
            evidence.validate()?;
            let source_matches = match evidence.source() {
                OutcomeEvidenceSource::Invocation(call) => {
                    self.invocation == Some(call) && self.adjudication_audit_digest.is_none()
                },
                OutcomeEvidenceSource::Reconciliation(call) => {
                    self.query == Some(call) && self.adjudication_audit_digest.is_none()
                },
                OutcomeEvidenceSource::Adjudication(_) => self.adjudication_audit_digest.is_some(),
            };
            if !source_matches {
                return Err(violation(OperationProtocolViolation::InconsistentState));
            }
        } else if self.adjudication_audit_digest.is_some() {
            return Err(violation(OperationProtocolViolation::InconsistentState));
        }
        Ok(())
    }

    /// Immutable pinned destination contract.
    pub const fn contract(&self) -> &PreparedEffectContract {
        &self.contract
    }
    /// Monotone protocol revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Current durable phase.
    pub const fn phase(&self) -> EffectPhase {
        self.phase
    }
    /// Backend preparation time in Unix milliseconds.
    pub const fn prepared_at_ms(&self) -> i64 {
        self.prepared_at_ms
    }
    /// Issued invocation permits, whether or not their call crossed.
    pub const fn invocations(&self) -> u32 {
        self.invocations
    }
    /// Issued permits whose call was durably proven not to cross the provider
    /// boundary; they spend no policy budget.
    pub const fn not_crossed(&self) -> u32 {
        self.not_crossed
    }
    /// Issued permits whose call may have reached the provider, including an
    /// outstanding one; these spend the policy's invocation budget.
    pub const fn crossed_invocations(&self) -> u32 {
        self.invocations.saturating_sub(self.not_crossed)
    }
    /// Consumed reconciliation permits.
    pub const fn queries(&self) -> u32 {
        self.queries
    }
    /// Most recent invocation identity.
    pub const fn invocation(&self) -> Option<OperationCallId> {
        self.invocation
    }
    /// Most recent invocation disposition.
    pub const fn disposition(&self) -> Option<InvocationDisposition> {
        self.disposition
    }
    /// Outstanding or completed reconciliation identity.
    pub const fn query(&self) -> Option<OperationCallId> {
        self.query
    }
    /// Exact terminal evidence when resolved.
    pub const fn evidence(&self) -> Option<&FrozenOutcomeEvidence> {
        self.evidence.as_ref()
    }
    /// Commitment to the operator audit note for adjudicated evidence.
    pub const fn adjudication_audit_digest(&self) -> Option<&[u8; 32]> {
        self.adjudication_audit_digest.as_ref()
    }
    /// Provider idempotency key recorded at preparation; immutable afterwards.
    pub const fn provider_key(&self) -> Option<ProviderIdempotencyKey> {
        self.provider_key
    }
}

impl TryFrom<OperationProtocolRecordWire> for OperationProtocolRecord {
    type Error = OperationLedgerError;

    fn try_from(wire: OperationProtocolRecordWire) -> Result<Self, Self::Error> {
        let record = Self {
            version: wire.version,
            contract: wire.contract,
            revision: wire.revision,
            phase: wire.phase,
            prepared_at_ms: wire.prepared_at_ms,
            invocations: wire.invocations,
            not_crossed: wire
                .not_crossed
                .unwrap_or_else(|| legacy_not_crossed(wire.disposition)),
            queries: wire.queries,
            invocation: wire.invocation,
            disposition: wire.disposition,
            query: wire.query,
            evidence: wire.evidence,
            adjudication_audit_digest: wire.adjudication_audit_digest,
            provider_key: wire.provider_key,
        };
        record.validate()?;
        Ok(record)
    }
}

impl<'de> Deserialize<'de> for OperationProtocolRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(OperationProtocolRecordWire::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

/// Named construction of a complete, validated protocol record.
#[derive(Debug)]
#[must_use]
pub struct OperationProtocolRecordBuilder {
    record: OperationProtocolRecord,
}

impl OperationProtocolRecordBuilder {
    /// Set the monotone protocol revision.
    pub const fn revision(mut self, revision: u64) -> Self {
        self.record.revision = revision;
        self
    }
    /// Set the durable phase.
    pub const fn phase(mut self, phase: EffectPhase) -> Self {
        self.record.phase = phase;
        self
    }
    /// Set invocation consumption and its most recent identity.
    pub const fn invocations(mut self, count: u32, call: Option<OperationCallId>) -> Self {
        self.record.invocations = count;
        self.record.invocation = call;
        self
    }
    /// Set how many issued permits were durably proven not to cross.
    pub const fn not_crossed(mut self, count: u32) -> Self {
        self.record.not_crossed = count;
        self
    }
    /// Set reconciliation consumption and its most recent identity.
    pub const fn queries(mut self, count: u32, call: Option<OperationCallId>) -> Self {
        self.record.queries = count;
        self.record.query = call;
        self
    }
    /// Set the immutable boundary disposition.
    pub const fn disposition(mut self, disposition: Option<InvocationDisposition>) -> Self {
        self.record.disposition = disposition;
        self
    }
    /// Set terminal evidence and its optional adjudication audit commitment.
    pub fn evidence(
        mut self,
        evidence: Option<FrozenOutcomeEvidence>,
        adjudication_audit_digest: Option<[u8; 32]>,
    ) -> Self {
        self.record.evidence = evidence;
        self.record.adjudication_audit_digest = adjudication_audit_digest;
        self
    }
    /// Set the provider idempotency key recorded at preparation.
    ///
    /// Adapters set it only when the record is first prepared; every later
    /// transition rebuilds from the stored record and so retains it.
    pub const fn provider_key(mut self, provider_key: Option<ProviderIdempotencyKey>) -> Self {
        self.record.provider_key = provider_key;
        self
    }
    /// Finish construction only when the complete record is coherent.
    ///
    /// # Errors
    /// Returns a specific [`OperationProtocolViolation`] for invalid state.
    pub fn build(self) -> Result<OperationProtocolRecord, OperationLedgerError> {
        self.record.validate()?;
        Ok(self.record)
    }
}

/// One explicit operation-owner transition; none is generic row replacement.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum OperationCommand {
    /// Issue a fresh effect permit after bounded admission.
    GrantInvocation {
        /// Last acknowledged protocol revision.
        expected_revision: u64,
    },
    /// Record the exact current call's boundary classification.
    RecordDisposition {
        /// Exact invocation being explained.
        invocation: OperationCallId,
        /// Immutable trusted boundary classification.
        disposition: InvocationDisposition,
    },
    /// Issue an authenticated read-only query permit.
    GrantReconciliation {
        /// Last acknowledged protocol revision.
        expected_revision: u64,
    },
    /// A query established no authoritative outcome; it grants no effect retry.
    RecordReconciliationInconclusive {
        /// Exact outstanding authenticated query.
        query: OperationCallId,
    },
    /// Atomically persist exact evidence, terminal state and owner journal.
    RecordOutcome(FrozenOutcomeEvidence),
    /// Exhausted/expired/unexplained effect; never permits effect re-invocation.
    MarkUnknown {
        /// Last acknowledged protocol revision.
        expected_revision: u64,
    },
}

/// Successful transition projection. Only a fresh acknowledged grant response
/// authorizes the effect driver's private control flow to call a provider.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum OperationAdvance {
    /// Newly consumed effect permit.
    Granted {
        /// Fresh effect call identity.
        call: OperationCallId,
        /// Backend clock when this permit was durably authorized.
        authorized_at_ms: i64,
        /// Exact state committed with this grant.
        record: super::OperationRecord,
    },
    /// Newly consumed read-only permit.
    ReconciliationGranted {
        /// Fresh read-only call identity.
        call: OperationCallId,
        /// Backend clock when this permit was durably authorized.
        authorized_at_ms: i64,
        /// Exact state committed with this grant.
        record: super::OperationRecord,
    },
    /// Durable transition or exact idempotent recommit.
    Recorded(super::OperationRecord),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_evidence_is_bounded_exact_and_payload_redacted() {
        let source = OutcomeEvidenceSource::Invocation(OperationCallId::from_bytes([1; 16]));
        let evidence = FrozenOutcomeEvidence::v1_json(
            source,
            KnownOutcome::Succeeded,
            b"{\"result\":\"evidence-canary\"}".to_vec(),
        )
        .unwrap();
        assert!(!format!("{evidence:?}").contains("evidence-canary"));
        let mut corrupt = serde_json::to_value(&evidence).unwrap();
        corrupt["digest"][0] = serde_json::json!(evidence.digest()[0] ^ 1);
        let error = serde_json::from_value::<FrozenOutcomeEvidence>(corrupt).unwrap_err();
        assert!(!error.to_string().contains("evidence-canary"));
        assert!(
            FrozenOutcomeEvidence::v1_json(source, KnownOutcome::OutcomeUnknown, b"{}".to_vec())
                .is_err()
        );
        assert!(
            FrozenOutcomeEvidence::v1_json(source, KnownOutcome::Succeeded, vec![b' '; 1_048_577])
                .is_err()
        );
        assert!(
            FrozenOutcomeEvidence::v1_json(
                source,
                KnownOutcome::Succeeded,
                b"provider-secret-invalid-json".to_vec()
            )
            .is_err()
        );
    }

    #[test]
    fn decoded_policy_cannot_smuggle_unbounded_or_unknown_guarantees() {
        let policy = PreparedEffectPolicy::builder(DestinationCapability::StableKey)
            .maximum_invocations(2)
            .maximum_queries(2)
            .recovery_window(Duration::from_secs(1))
            .stable_key_window(Duration::from_millis(500))
            .build()
            .unwrap();
        let mut encoded = serde_json::to_value(&policy).unwrap();
        encoded["max_invocations"] = serde_json::json!(0);
        let error = serde_json::from_value::<PreparedEffectPolicy>(encoded.clone()).unwrap_err();
        assert!(error.to_string().contains("invocation limit"));
        encoded["unrecognized_guarantee"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PreparedEffectPolicy>(encoded).is_err());
        assert!(
            PreparedEffectPolicy::builder(DestinationCapability::StableKey)
                .maximum_invocations(1)
                .maximum_queries(0)
                .recovery_window(Duration::from_secs(1))
                .build()
                .is_err()
        );
        assert!(
            PreparedEffectPolicy::builder(DestinationCapability::Opaque)
                .maximum_invocations(1)
                .maximum_queries(1)
                .recovery_window(Duration::from_secs(1))
                .build()
                .is_err()
        );
    }

    #[test]
    fn named_policy_builder_requires_every_limit() {
        let missing_queries = PreparedEffectPolicy::builder(DestinationCapability::Opaque)
            .maximum_invocations(1)
            .recovery_window(Duration::from_secs(1))
            .build();

        assert_eq!(
            missing_queries,
            Err(violation(OperationProtocolViolation::QueryLimit))
        );
    }

    #[test]
    fn decoded_record_rejects_inconsistent_terminal_state() {
        let policy = PreparedEffectPolicy::builder(DestinationCapability::Opaque)
            .maximum_invocations(1)
            .maximum_queries(0)
            .recovery_window(Duration::from_secs(1))
            .build()
            .unwrap();
        let contract =
            PreparedEffectContract::new(RequestFingerprint::new(1, [7; 32]), policy).unwrap();
        let record = OperationProtocolRecord::prepared(contract, 0)
            .build()
            .unwrap();
        let mut encoded = serde_json::to_value(record).unwrap();
        encoded["phase"] = serde_json::json!("Resolved");

        let error = serde_json::from_value::<OperationProtocolRecord>(encoded).unwrap_err();
        assert!(error.to_string().contains("phase and retained evidence"));
    }
}
