//! Backend-independent operation-ledger decisions.
//!
//! The in-memory reference model, SQLite, and PostgreSQL must answer every
//! ledger question identically, so the questions are answered once here. Row
//! plumbing stays in each adapter; the decisions do not.
//!
//! Durable state text is the vocabulary ordered migration 0045 constrains with
//! `CHECK` clauses. Destination text conversions live with the typed value.

use nebula_core::{OperationCallId, OperationId};
use nebula_storage_port::dto::{
    EffectPhase, FrozenOutcomeEvidence, InvocationDisposition, OperationAdvance, OperationCommand,
    OperationProtocolRecord, OutcomeEvidenceSource,
};
use nebula_storage_port::{
    AttemptGeneration, DestinationCapability, EffectSlotBinding, EffectSlotId, KnownOutcome,
    OperationLedgerError, OperationRecord, OperationState, PrepareOutcome, PreparedOperation,
    RequestFingerprint,
};

/// Durable text of each operation state.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_PREPARED: &str = "prepared";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_SUCCEEDED: &str = "succeeded";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_FAILED: &str = "failed";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_OUTCOME_UNKNOWN: &str = "outcome_unknown";

/// Render an operation state as the text migration 0045 admits.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const fn state_text(state: OperationState) -> &'static str {
    match state {
        OperationState::Prepared => STATE_PREPARED,
        OperationState::Succeeded => STATE_SUCCEEDED,
        OperationState::Failed => STATE_FAILED,
        _ => STATE_OUTCOME_UNKNOWN,
    }
}

/// Parse durable state text, rejecting vocabulary outside the schema.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn state_from_text(text: &str) -> Option<OperationState> {
    match text {
        STATE_PREPARED => Some(OperationState::Prepared),
        STATE_SUCCEEDED => Some(OperationState::Succeeded),
        STATE_FAILED => Some(OperationState::Failed),
        STATE_OUTCOME_UNKNOWN => Some(OperationState::OutcomeUnknown),
        _ => None,
    }
}

/// The durable state one known outcome resolves to.
pub(crate) const fn outcome_state(outcome: KnownOutcome) -> OperationState {
    match outcome {
        KnownOutcome::Succeeded => OperationState::Succeeded,
        KnownOutcome::Failed => OperationState::Failed,
        _ => OperationState::OutcomeUnknown,
    }
}

/// Whether two fingerprints describe the same canonical request.
///
/// The version must match as well as the digest: digests produced under
/// different canonicalization rules are not comparable, so a version change
/// reads as a mismatch rather than risking a false match that would hand two
/// different requests one operation identity.
pub(crate) fn fingerprints_match(
    stored: RequestFingerprint,
    candidate: RequestFingerprint,
) -> bool {
    stored.version() == candidate.version() && stored.digest() == candidate.digest()
}

/// Decide what preparing `binding` against an existing record must do.
///
/// # Errors
///
/// Returns [`OperationLedgerError::OperationMismatch`] when the slot is bound
/// to a different canonical request. The caller writes nothing in that case:
/// reusing a slot for a different request would give two distinct effects one
/// operation identity.
pub(crate) fn decide_prepare(
    slot_id: EffectSlotId,
    stored: &OperationRecord,
    binding: &EffectSlotBinding<'_>,
) -> Result<PrepareOutcome, OperationLedgerError> {
    if !fingerprints_match(stored.fingerprint(), binding.fingerprint) {
        return Err(OperationLedgerError::OperationMismatch { slot_id });
    }
    if let Some(protocol) = stored.protocol()
        && protocol.contract() != binding.contract
    {
        return Err(OperationLedgerError::OperationMismatch { slot_id });
    }
    // The original binding is returned wholesale — including the attempt
    // generation and destination recorded at prepare time. A later attempt
    // re-preparing the same slot inherits the first attempt's operation
    // identity, which is exactly what lets a restarted worker reach the
    // provider under one identity.
    Ok(PrepareOutcome::Replayed(stored.operation()))
}

/// Bounded version-one protocol initialized only for newly inserted operations.
pub(crate) fn initial_protocol(
    binding: &EffectSlotBinding<'_>,
    now_ms: i64,
) -> Result<OperationProtocolRecord, OperationLedgerError> {
    binding.contract.validate()?;
    if binding.destination != binding.contract.policy().capability() {
        return Err(OperationLedgerError::InvalidProtocol);
    }
    now_ms
        .checked_add(
            i64::try_from(binding.contract.policy().recovery_window_ms())
                .map_err(|_| OperationLedgerError::InvalidProtocol)?,
        )
        .ok_or(OperationLedgerError::InvalidProtocol)?;
    OperationProtocolRecord::prepared(binding.contract.clone(), now_ms).build()
}

/// One fully decided transition, ready for infallible in-memory writes or SQL.
#[derive(Debug)]
pub(crate) struct ProtocolDecision {
    pub record: OperationRecord,
    pub response: OperationAdvance,
    pub changed: bool,
    pub journal: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy)]
enum GrantedCall {
    Invocation,
    Reconciliation,
}

struct ProtocolTransition {
    original: OperationProtocolRecord,
    revision: u64,
    phase: EffectPhase,
    invocations: u32,
    queries: u32,
    invocation: Option<OperationCallId>,
    disposition: Option<InvocationDisposition>,
    query: Option<OperationCallId>,
    outcome_evidence: Option<FrozenOutcomeEvidence>,
    target_state: OperationState,
    granted_call: Option<GrantedCall>,
}

impl ProtocolTransition {
    fn new(protocol: OperationProtocolRecord, target_state: OperationState) -> Self {
        Self {
            revision: protocol.revision(),
            phase: protocol.phase(),
            invocations: protocol.invocations(),
            queries: protocol.queries(),
            invocation: protocol.invocation(),
            disposition: protocol.disposition(),
            query: protocol.query(),
            outcome_evidence: protocol.evidence().cloned(),
            original: protocol,
            target_state,
            granted_call: None,
        }
    }

    const fn is_terminal(&self) -> bool {
        matches!(self.phase, EffectPhase::Resolved)
    }

    fn is_within_window(&self, window_ms: u64, now_ms: i64) -> bool {
        i64::try_from(window_ms)
            .ok()
            .and_then(|window_ms| self.original.prepared_at_ms().checked_add(window_ms))
            .is_some_and(|deadline_ms| now_ms < deadline_ms)
    }

    fn has_changes(&self) -> bool {
        self.phase != self.original.phase()
            || self.invocations != self.original.invocations()
            || self.queries != self.original.queries()
            || self.invocation != self.original.invocation()
            || self.disposition != self.original.disposition()
            || self.query != self.original.query()
            || self.outcome_evidence.as_ref() != self.original.evidence()
    }
}

fn grant_invocation(
    transition: &mut ProtocolTransition,
    expected_revision: u64,
    now_ms: i64,
    fresh_call: OperationCallId,
) -> Result<(), OperationLedgerError> {
    if expected_revision != transition.revision
        || transition.is_terminal()
        || transition.query.is_some()
        || !matches!(
            transition.phase,
            EffectPhase::Prepared | EffectPhase::BeforeBoundary | EffectPhase::Ambiguous
        )
    {
        return Err(OperationLedgerError::ProtocolConflict);
    }

    let policy = transition.original.contract().policy();
    let stable_key_is_valid = match policy.capability() {
        DestinationCapability::StableKey => policy
            .stable_window_ms()
            .is_some_and(|window_ms| transition.is_within_window(window_ms, now_ms)),
        _ => transition.phase != EffectPhase::Ambiguous,
    };
    if transition.invocations >= policy.max_invocations()
        || !transition.is_within_window(policy.recovery_window_ms(), now_ms)
        || !stable_key_is_valid
    {
        transition.phase = EffectPhase::OutcomeUnknown;
        transition.target_state = OperationState::OutcomeUnknown;
        return Ok(());
    }

    transition.invocations += 1;
    transition.invocation = Some(fresh_call);
    transition.disposition = None;
    transition.phase = EffectPhase::InvocationOutstanding;
    transition.granted_call = Some(GrantedCall::Invocation);
    Ok(())
}

fn record_invocation_disposition(
    transition: &mut ProtocolTransition,
    reported_invocation: OperationCallId,
    reported_disposition: InvocationDisposition,
) -> Result<(), OperationLedgerError> {
    if transition.is_terminal() || transition.invocation != Some(reported_invocation) {
        return Err(OperationLedgerError::ProtocolConflict);
    }
    if let Some(recorded_disposition) = transition.disposition {
        if recorded_disposition != reported_disposition {
            return Err(OperationLedgerError::ProtocolConflict);
        }
        return Ok(());
    }
    if transition.phase != EffectPhase::InvocationOutstanding {
        return Err(OperationLedgerError::ProtocolConflict);
    }

    let next_phase = match reported_disposition {
        InvocationDisposition::BeforeBoundary => EffectPhase::BeforeBoundary,
        InvocationDisposition::Ambiguous
            if transition.original.contract().policy().capability()
                == DestinationCapability::StableKey =>
        {
            EffectPhase::Ambiguous
        },
        InvocationDisposition::Ambiguous => {
            transition.target_state = OperationState::OutcomeUnknown;
            EffectPhase::OutcomeUnknown
        },
        _ => return Err(OperationLedgerError::ProtocolConflict),
    };
    transition.disposition = Some(reported_disposition);
    transition.phase = next_phase;
    Ok(())
}

fn grant_reconciliation(
    transition: &mut ProtocolTransition,
    expected_revision: u64,
    now_ms: i64,
    fresh_call: OperationCallId,
) -> Result<(), OperationLedgerError> {
    if expected_revision != transition.revision
        || transition.phase != EffectPhase::OutcomeUnknown
        || transition.query.is_some()
    {
        return Err(OperationLedgerError::ProtocolConflict);
    }

    let policy = transition.original.contract().policy();
    if transition.queries >= policy.max_queries()
        || !transition.is_within_window(policy.recovery_window_ms(), now_ms)
    {
        return Err(OperationLedgerError::RecoveryExhausted);
    }
    transition.queries += 1;
    transition.query = Some(fresh_call);
    transition.granted_call = Some(GrantedCall::Reconciliation);
    Ok(())
}

fn record_reconciliation_inconclusive(
    transition: &mut ProtocolTransition,
    completed_query: OperationCallId,
) -> Result<(), OperationLedgerError> {
    if transition.is_terminal() || transition.query != Some(completed_query) {
        return Err(OperationLedgerError::ProtocolConflict);
    }
    transition.query = None;
    transition.phase = EffectPhase::OutcomeUnknown;
    transition.target_state = OperationState::OutcomeUnknown;
    Ok(())
}

fn record_outcome(
    transition: &mut ProtocolTransition,
    stored: &OperationRecord,
    reported_evidence: &FrozenOutcomeEvidence,
) -> Result<(), OperationLedgerError> {
    reported_evidence.validate()?;
    if let Some(recorded_evidence) = &transition.outcome_evidence {
        if recorded_evidence == reported_evidence {
            return Ok(());
        }
        if outcome_state(reported_evidence.outcome()) != stored.state() {
            return Err(OperationLedgerError::OutcomeAlreadyRecorded {
                slot_id: stored.operation().slot_id(),
                recorded: stored.state(),
            });
        }
        return Err(OperationLedgerError::ProtocolConflict);
    }
    if transition.is_terminal() {
        return Err(OperationLedgerError::ProtocolConflict);
    }

    let evidence_is_permitted = match reported_evidence.source() {
        OutcomeEvidenceSource::Invocation(call) => {
            transition.invocation == Some(call)
                && transition.phase == EffectPhase::InvocationOutstanding
                && transition.disposition.is_none()
        },
        OutcomeEvidenceSource::Reconciliation(call) => transition.query == Some(call),
        OutcomeEvidenceSource::Adjudication(_) => false,
        _ => false,
    };
    if !evidence_is_permitted {
        return Err(OperationLedgerError::ProtocolConflict);
    }

    transition.outcome_evidence = Some(reported_evidence.clone());
    transition.phase = EffectPhase::Resolved;
    transition.target_state = outcome_state(reported_evidence.outcome());
    Ok(())
}

fn mark_outcome_unknown(
    transition: &mut ProtocolTransition,
    expected_revision: u64,
) -> Result<(), OperationLedgerError> {
    if expected_revision != transition.revision || transition.is_terminal() {
        return Err(OperationLedgerError::ProtocolConflict);
    }
    transition.phase = EffectPhase::OutcomeUnknown;
    transition.target_state = OperationState::OutcomeUnknown;
    Ok(())
}

fn finalize_transition(
    stored: &OperationRecord,
    mut transition: ProtocolTransition,
    fresh_call: OperationCallId,
) -> Result<ProtocolDecision, OperationLedgerError> {
    let changed = transition.has_changes();
    if changed {
        transition.revision = transition
            .revision
            .checked_add(1)
            .ok_or(OperationLedgerError::InvalidProtocol)?;
    }
    let adjudication_audit_digest = transition.original.adjudication_audit_digest().copied();
    let protocol = transition
        .original
        .rebuild()
        .revision(transition.revision)
        .phase(transition.phase)
        .invocations(transition.invocations, transition.invocation)
        .queries(transition.queries, transition.query)
        .disposition(transition.disposition)
        .evidence(transition.outcome_evidence, adjudication_audit_digest)
        .build()?;
    let journal = (changed && transition.target_state != stored.state()).then(|| serde_json::json!({
        "kind":"operation_outcome", "version":1, "slot_id":stored.operation().slot_id().to_string(),
        "operation_id":stored.operation().operation_id().to_string(), "protocol_revision":protocol.revision(),
        "outcome": match transition.target_state { OperationState::Succeeded => "succeeded", OperationState::Failed => "failed", _ => "outcome_unknown" }
    }));
    let record = OperationRecord::new(
        stored.operation(),
        stored.fingerprint(),
        transition.target_state,
    )
    .with_protocol(protocol);
    let response = match transition.granted_call {
        Some(GrantedCall::Invocation) => OperationAdvance::Granted {
            call: fresh_call,
            record: record.clone(),
        },
        Some(GrantedCall::Reconciliation) => OperationAdvance::ReconciliationGranted {
            call: fresh_call,
            record: record.clone(),
        },
        None => OperationAdvance::Recorded(record.clone()),
    };
    Ok(ProtocolDecision {
        record,
        response,
        changed,
        journal,
    })
}

/// Shared finite protocol; the adapter must hold the actual live execution fence.
pub(crate) fn decide_advance(
    stored: &OperationRecord,
    command: &OperationCommand,
    now_ms: i64,
    fresh_call: OperationCallId,
) -> Result<ProtocolDecision, OperationLedgerError> {
    let protocol = stored
        .protocol()
        .cloned()
        .ok_or(OperationLedgerError::ProtocolConflict)?;
    validate_record(stored)?;
    let mut transition = ProtocolTransition::new(protocol, stored.state());
    match command {
        OperationCommand::GrantInvocation { expected_revision } => {
            grant_invocation(&mut transition, *expected_revision, now_ms, fresh_call)?;
        },
        OperationCommand::RecordDisposition {
            invocation: reported_invocation,
            disposition: reported_disposition,
        } => {
            record_invocation_disposition(
                &mut transition,
                *reported_invocation,
                *reported_disposition,
            )?;
        },
        OperationCommand::GrantReconciliation { expected_revision } => {
            grant_reconciliation(&mut transition, *expected_revision, now_ms, fresh_call)?;
        },
        OperationCommand::RecordReconciliationInconclusive {
            query: completed_query,
        } => {
            record_reconciliation_inconclusive(&mut transition, *completed_query)?;
        },
        OperationCommand::RecordOutcome(reported_evidence) => {
            record_outcome(&mut transition, stored, reported_evidence)?;
        },
        OperationCommand::MarkUnknown { expected_revision } => {
            mark_outcome_unknown(&mut transition, *expected_revision)?;
        },
        _ => return Err(OperationLedgerError::ProtocolConflict),
    }
    finalize_transition(stored, transition, fresh_call)
}

pub(crate) fn validate_protocol(
    protocol: &OperationProtocolRecord,
) -> Result<(), OperationLedgerError> {
    protocol.validate()
}

pub(crate) fn validate_record(record: &OperationRecord) -> Result<(), OperationLedgerError> {
    use nebula_storage_port::dto::EffectPhase;
    if let Some(protocol) = record.protocol() {
        validate_protocol(protocol)?;
        let expected = match protocol.phase() {
            EffectPhase::OutcomeUnknown => OperationState::OutcomeUnknown,
            EffectPhase::Resolved => outcome_state(
                protocol
                    .evidence()
                    .ok_or(OperationLedgerError::InvalidProtocol)?
                    .outcome(),
            ),
            _ => OperationState::Prepared,
        };
        if expected != record.state()
            || protocol.contract().policy().capability() != record.operation().destination()
        {
            return Err(OperationLedgerError::CorruptRecord {
                slot_id: record.operation().slot_id(),
            });
        }
    }
    Ok(())
}

/// Privileged adjudication uses the same immutable evidence rules, never a grant.
pub(crate) fn decide_adjudicate_protocol(
    stored: &OperationRecord,
    evidence: &FrozenOutcomeEvidence,
    audit: &str,
) -> Result<ProtocolDecision, OperationLedgerError> {
    use nebula_storage_port::dto::{EffectPhase, OperationAdvance, OutcomeEvidenceSource};
    use sha2::{Digest as _, Sha256};
    evidence.validate()?;
    if audit.trim().is_empty()
        || audit.len() > 4096
        || !matches!(evidence.source(), OutcomeEvidenceSource::Adjudication(_))
    {
        return Err(OperationLedgerError::InvalidProtocol);
    }
    let protocol = stored
        .protocol()
        .cloned()
        .ok_or(OperationLedgerError::ProtocolConflict)?;
    validate_record(stored)?;
    let audit_digest: [u8; 32] = Sha256::digest(audit.as_bytes()).into();
    if let Some(recorded) = protocol.evidence() {
        if recorded == evidence && protocol.adjudication_audit_digest() == Some(&audit_digest) {
            return Ok(ProtocolDecision {
                record: stored.clone(),
                response: OperationAdvance::Recorded(stored.clone()),
                changed: false,
                journal: None,
            });
        }
        return Err(OperationLedgerError::OutcomeAlreadyRecorded {
            slot_id: stored.operation().slot_id(),
            recorded: stored.state(),
        });
    }
    if protocol.phase() != EffectPhase::OutcomeUnknown {
        return Err(OperationLedgerError::ProtocolConflict);
    }
    let revision = protocol
        .revision()
        .checked_add(1)
        .ok_or(OperationLedgerError::InvalidProtocol)?;
    let protocol = protocol
        .rebuild()
        .revision(revision)
        .phase(EffectPhase::Resolved)
        .evidence(Some(evidence.clone()), Some(audit_digest))
        .build()?;
    let journal = serde_json::json!({"kind":"operation_adjudicated", "version":1,
        "slot_id":stored.operation().slot_id().to_string(), "operation_id":stored.operation().operation_id().to_string(),
        "protocol_revision":protocol.revision(), "outcome":if evidence.outcome() == KnownOutcome::Succeeded {"succeeded"} else {"failed"}});
    let record = OperationRecord::new(
        stored.operation(),
        stored.fingerprint(),
        outcome_state(evidence.outcome()),
    )
    .with_protocol(protocol);
    Ok(ProtocolDecision {
        record: record.clone(),
        response: OperationAdvance::Recorded(record),
        changed: true,
        journal: Some(journal),
    })
}

/// Decode only a bounded protocol; legacy rows retain their absence.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_protocol(
    bytes: Option<&str>,
) -> Result<Option<OperationProtocolRecord>, OperationLedgerError> {
    bytes
        .map(|bytes| {
            if bytes.len() > 5_300_000 {
                return Err(OperationLedgerError::InvalidProtocol);
            }
            let protocol =
                serde_json::from_str(bytes).map_err(|_| OperationLedgerError::InvalidProtocol)?;
            validate_protocol(&protocol)?;
            Ok(protocol)
        })
        .transpose()
}

/// Called under the execution owner's lock using its authoritative clock.
pub(crate) fn require_live_lease(
    fencing: nebula_storage_port::FencingToken,
    current: u64,
    live: bool,
) -> Result<(), OperationLedgerError> {
    if fencing.generation() != current || !live {
        return Err(OperationLedgerError::ExecutionLeaseRejected);
    }
    Ok(())
}

/// Compose a record projection from decoded durable columns.
pub(crate) fn compose_record(
    slot_id: EffectSlotId,
    operation_id: OperationId,
    attempt_generation: AttemptGeneration,
    destination: DestinationCapability,
    fingerprint: RequestFingerprint,
    state: OperationState,
) -> OperationRecord {
    OperationRecord::new(
        PreparedOperation::new(slot_id, operation_id, attempt_generation, destination),
        fingerprint,
        state,
    )
}

/// Stable label naming one ledger rejection, so every adapter reports the same
/// outcome vocabulary on its spans and counters.
pub(crate) const fn error_label(error: &OperationLedgerError) -> &'static str {
    match *error {
        OperationLedgerError::InvalidProtocol => "invalid_protocol",
        OperationLedgerError::ProtocolConflict => "protocol_conflict",
        OperationLedgerError::RecoveryExhausted => "recovery_exhausted",
        OperationLedgerError::InvalidAttemptGeneration => "invalid_attempt_generation",
        OperationLedgerError::ExecutionLeaseRejected => "execution_lease_rejected",
        OperationLedgerError::OperationMismatch { .. } => "operation_mismatch",
        OperationLedgerError::SlotUnprepared { .. } => "slot_unprepared",
        OperationLedgerError::TenantDenied => "tenant_denied",
        OperationLedgerError::OutcomeAlreadyRecorded { .. } => "outcome_already_recorded",
        OperationLedgerError::CorruptRecord { .. } => "corrupt_record",
        OperationLedgerError::Unavailable => "unavailable",
        OperationLedgerError::AcknowledgementUnknown => "acknowledgement_unknown",
        // The port marks this error `#[non_exhaustive]`, so a wildcard is
        // required. Naming the gap beats folding an unrecognised rejection
        // into a neighbouring bucket, where a dashboard would read it as a
        // rejection this build understands.
        _ => "unclassified",
    }
}

/// Stable transition labels distinguish fresh grants from durable result replay.
pub(crate) fn advance_label(
    result: &Result<OperationAdvance, OperationLedgerError>,
) -> &'static str {
    use nebula_storage_port::dto::OperationAdvance;
    match result {
        Ok(OperationAdvance::Granted { .. }) => "invocation_granted",
        Ok(OperationAdvance::ReconciliationGranted { .. }) => "reconciliation_granted",
        Ok(OperationAdvance::Recorded(_)) => "recorded",
        Ok(_) => "unclassified",
        Err(error) => error_label(error),
    }
}

/// Stable outcome label for one prepare.
pub(crate) const fn prepare_label(
    result: &Result<PrepareOutcome, OperationLedgerError>,
) -> &'static str {
    match *result {
        Ok(PrepareOutcome::Prepared(_)) => "prepared",
        Ok(PrepareOutcome::Replayed(_)) => "replayed",
        Err(ref error) => error_label(error),
    }
}

/// Checked portable representation; attempt provenance never grants authority.
pub(crate) fn stored_attempt_generation(
    generation: AttemptGeneration,
) -> Result<i64, OperationLedgerError> {
    i64::try_from(generation.get()).map_err(|_| OperationLedgerError::InvalidAttemptGeneration)
}

/// Stable outcome label for one natural-key read.
pub(crate) const fn occurrence_read_label(
    result: &Result<Option<OperationRecord>, OperationLedgerError>,
) -> &'static str {
    match result {
        Ok(Some(_)) => "read",
        Ok(None) => "absent",
        Err(error) => error_label(error),
    }
}

/// Stable outcome label for one exact read.
pub(crate) const fn read_label(
    result: &Result<OperationRecord, OperationLedgerError>,
) -> &'static str {
    match *result {
        Ok(_) => "read",
        Err(ref error) => error_label(error),
    }
}

/// Stable outcome label for one fenced commit or adjudication.
pub(crate) const fn write_label(result: &Result<(), OperationLedgerError>) -> &'static str {
    match *result {
        Ok(()) => "committed",
        Err(ref error) => error_label(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> nebula_storage_port::dto::PreparedEffectContract {
        nebula_storage_port::dto::PreparedEffectContract::new(
            RequestFingerprint::new(1, [9; 32]),
            nebula_storage_port::dto::PreparedEffectPolicy::builder(
                DestinationCapability::StableKey,
            )
            .maximum_invocations(3)
            .maximum_queries(3)
            .recovery_window(std::time::Duration::from_mins(1))
            .stable_key_window(std::time::Duration::from_mins(1))
            .build()
            .unwrap(),
        )
        .unwrap()
    }

    fn slot() -> EffectSlotId {
        EffectSlotId::from_storage_bytes([0x11; 16])
    }

    fn record(state: OperationState, generation: u64) -> OperationRecord {
        compose_record(
            slot(),
            OperationId::from_bytes([0x22; 16]),
            AttemptGeneration::new(generation),
            DestinationCapability::StableKey,
            RequestFingerprint::new(1, [0x33; 32]),
            state,
        )
    }

    /// Durable text exists only where a SQL backend writes it.
    #[cfg(any(feature = "sqlite", feature = "postgres"))]
    #[test]
    fn durable_vocabulary_round_trips() {
        for destination in [
            DestinationCapability::StableKey,
            DestinationCapability::Reconcilable,
            DestinationCapability::Opaque,
        ] {
            assert_eq!(
                DestinationCapability::try_from(<&'static str>::from(destination)),
                Ok(destination)
            );
        }
        for state in [
            OperationState::Prepared,
            OperationState::Succeeded,
            OperationState::Failed,
            OperationState::OutcomeUnknown,
        ] {
            assert_eq!(state_from_text(state_text(state)), Some(state));
        }
        assert!(DestinationCapability::try_from("best_effort").is_err());
        assert_eq!(state_from_text("maybe"), None);
    }

    /// Digests computed under different rules are not comparable, so a version
    /// change must read as a mismatch rather than reuse an operation identity
    /// for a different request.
    #[test]
    fn a_digest_under_different_rules_is_not_the_same_request() {
        let stored = RequestFingerprint::new(1, [0x33; 32]);
        assert!(fingerprints_match(
            stored,
            RequestFingerprint::new(1, [0x33; 32])
        ));
        assert!(!fingerprints_match(
            stored,
            RequestFingerprint::new(2, [0x33; 32])
        ));
        assert!(!fingerprints_match(
            stored,
            RequestFingerprint::new(1, [0x34; 32])
        ));
    }

    #[test]
    fn a_later_attempt_inherits_the_original_operation_identity() {
        let stored = record(OperationState::Prepared, 0);
        let scope = nebula_storage_port::Scope::new("ws", "org");
        let binding = EffectSlotBinding {
            scope: &scope,
            execution_id: "exe",
            node_key: "node",
            occurrence: "once",
            attempt_generation: AttemptGeneration::new(7),
            fingerprint: RequestFingerprint::new(1, [0x33; 32]),
            destination: DestinationCapability::Opaque,
            contract: &contract(),
        };

        let outcome = decide_prepare(slot(), &stored, &binding)
            .expect("a matching fingerprint replays the original binding");
        assert_eq!(
            outcome.operation().operation_id(),
            stored.operation().operation_id(),
            "a restarted worker must reach the provider under one identity"
        );
        assert_eq!(
            outcome.operation().destination(),
            DestinationCapability::StableKey,
            "the guarantee recorded at prepare time governs, not the caller's current view"
        );
    }

    #[test]
    fn a_different_request_on_the_same_slot_fails_closed() {
        let stored = record(OperationState::Prepared, 0);
        let scope = nebula_storage_port::Scope::new("ws", "org");
        let binding = EffectSlotBinding {
            scope: &scope,
            execution_id: "exe",
            node_key: "node",
            occurrence: "once",
            attempt_generation: AttemptGeneration::new(0),
            fingerprint: RequestFingerprint::new(1, [0x99; 32]),
            destination: DestinationCapability::StableKey,
            contract: &contract(),
        };

        assert_eq!(
            decide_prepare(slot(), &stored, &binding),
            Err(OperationLedgerError::OperationMismatch { slot_id: slot() })
        );
    }

    #[test]
    fn a_superseded_attempt_cannot_decide_the_current_one() {
        assert_eq!(
            require_live_lease(
                nebula_storage_port::FencingToken::from_generation(4),
                5,
                true
            ),
            Err(OperationLedgerError::ExecutionLeaseRejected)
        );
    }

    #[test]
    fn legacy_projection_cannot_grant_invocation_authority() {
        let legacy = record(OperationState::Prepared, 1);
        let result = decide_advance(
            &legacy,
            &OperationCommand::GrantInvocation {
                expected_revision: 0,
            },
            0,
            OperationCallId::from_bytes([1; 16]),
        );
        assert!(matches!(
            result,
            Err(OperationLedgerError::ProtocolConflict)
        ));
        assert_eq!(legacy.protocol(), None);
    }

    #[test]
    fn reconciliation_requires_unknown_outcome_without_an_outstanding_query() {
        use nebula_core::OperationCallId;
        use nebula_storage_port::dto::{EffectPhase, OperationCommand};

        let call = OperationCallId::from_bytes([1; 16]);
        let protocol = OperationProtocolRecord::prepared(contract(), 0)
            .revision(1)
            .phase(EffectPhase::InvocationOutstanding)
            .invocations(1, Some(call))
            .build()
            .unwrap();
        let outstanding_invocation =
            record(OperationState::Prepared, 0).with_protocol(protocol.clone());
        std::assert_matches!(
            decide_advance(
                &outstanding_invocation,
                &OperationCommand::GrantReconciliation {
                    expected_revision: 1,
                },
                1,
                OperationCallId::from_bytes([2; 16]),
            ),
            Err(OperationLedgerError::ProtocolConflict)
        );

        let protocol = protocol
            .rebuild()
            .revision(2)
            .phase(EffectPhase::OutcomeUnknown)
            .queries(1, Some(OperationCallId::from_bytes([3; 16])))
            .build()
            .unwrap();
        let outstanding_query =
            record(OperationState::OutcomeUnknown, 0).with_protocol(protocol.clone());
        std::assert_matches!(
            decide_advance(
                &outstanding_query,
                &OperationCommand::GrantReconciliation {
                    expected_revision: 2,
                },
                1,
                OperationCallId::from_bytes([4; 16]),
            ),
            Err(OperationLedgerError::ProtocolConflict)
        );

        std::assert_matches!(
            protocol
                .rebuild()
                .phase(EffectPhase::InvocationOutstanding)
                .build(),
            Err(OperationLedgerError::ProtocolViolation { .. })
        );
    }
}
