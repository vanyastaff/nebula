//! Backend-independent operation-ledger decisions.
//!
//! The in-memory reference model, SQLite, and PostgreSQL must answer every
//! ledger question identically, so the questions are answered once here. Row
//! plumbing stays in each adapter; the decisions do not.
//!
//! Durable state and destination text is the same vocabulary ordered migration
//! 0045 constrains with `CHECK` clauses. The constants below are the single
//! Rust-side definition of that vocabulary.

use nebula_storage_port::{
    AttemptGeneration, DestinationCapability, EffectSlotBinding, EffectSlotId, KnownOutcome,
    OperationId, OperationLedgerError, OperationRecord, OperationState, PrepareOutcome,
    PreparedOperation, RequestFingerprint,
};

/// Durable text of each destination guarantee.
///
/// Only a SQL backend spells these; the in-memory reference model holds the
/// typed values directly.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const DESTINATION_STABLE_KEY: &str = "stable_key";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const DESTINATION_RECONCILABLE: &str = "reconcilable";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const DESTINATION_OPAQUE: &str = "opaque";

/// Durable text of each operation state.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_PREPARED: &str = "prepared";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_SUCCEEDED: &str = "succeeded";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_FAILED: &str = "failed";
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const STATE_OUTCOME_UNKNOWN: &str = "outcome_unknown";

/// Render a destination guarantee as the text migration 0045 admits.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const fn destination_text(destination: DestinationCapability) -> &'static str {
    match destination {
        DestinationCapability::StableKey => DESTINATION_STABLE_KEY,
        DestinationCapability::Reconcilable => DESTINATION_RECONCILABLE,
        // A destination this build does not recognise offers no guarantee, so
        // it is treated as opaque rather than as the nearest known one:
        // guessing upward would authorize a re-invocation the destination
        // never promised to deduplicate.
        _ => DESTINATION_OPAQUE,
    }
}

/// Parse durable destination text, rejecting vocabulary outside the schema.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn destination_from_text(text: &str) -> Option<DestinationCapability> {
    match text {
        DESTINATION_STABLE_KEY => Some(DestinationCapability::StableKey),
        DESTINATION_RECONCILABLE => Some(DestinationCapability::Reconcilable),
        DESTINATION_OPAQUE => Some(DestinationCapability::Opaque),
        _ => None,
    }
}

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
    // The original binding is returned wholesale — including the attempt
    // generation and destination recorded at prepare time. A later attempt
    // re-preparing the same slot inherits the first attempt's operation
    // identity, which is exactly what lets a restarted worker reach the
    // provider under one identity.
    Ok(PrepareOutcome::Replayed(stored.operation()))
}

/// Decide what committing `outcome` against an existing record must do.
///
/// # Errors
///
/// Returns [`OperationLedgerError::StaleFence`] when the caller's attempt is
/// behind the durable binding, and
/// [`OperationLedgerError::OutcomeAlreadyRecorded`] when a *different* terminal
/// outcome exists.
pub(crate) fn decide_commit(
    slot_id: EffectSlotId,
    stored: &OperationRecord,
    attempt_generation: AttemptGeneration,
    outcome: KnownOutcome,
) -> Result<CommitDecision, OperationLedgerError> {
    let bound = stored.operation().attempt_generation();
    if attempt_generation < bound {
        return Err(OperationLedgerError::StaleFence {
            slot_id,
            current: bound,
        });
    }

    let target = outcome_state(outcome);
    match stored.state() {
        OperationState::Prepared => Ok(CommitDecision::Apply(target)),
        // Committing the same outcome again is idempotent, which is what lets
        // a caller whose acknowledgement was lost recommit the same frozen
        // evidence without inventing a second answer.
        current if current == target => Ok(CommitDecision::AlreadyRecorded),
        current => Err(OperationLedgerError::OutcomeAlreadyRecorded {
            slot_id,
            recorded: current,
        }),
    }
}

/// What a fenced outcome commit must write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitDecision {
    /// Move the record to this state.
    Apply(OperationState),
    /// The record already holds this outcome; write nothing.
    AlreadyRecorded,
}

/// Decide whether `outcome` may be adjudicated onto an existing record.
///
/// # Errors
///
/// Returns [`OperationLedgerError::OutcomeAlreadyRecorded`] when the operation
/// is not `OutcomeUnknown`. Adjudication resolves uncertainty; it never
/// overrules an answer the system determined for itself, and it cannot leave
/// the record still unknown.
pub(crate) fn decide_adjudication(
    slot_id: EffectSlotId,
    stored: &OperationRecord,
    outcome: KnownOutcome,
) -> Result<OperationState, OperationLedgerError> {
    if stored.state() != OperationState::OutcomeUnknown {
        return Err(OperationLedgerError::OutcomeAlreadyRecorded {
            slot_id,
            recorded: stored.state(),
        });
    }
    let target = outcome_state(outcome);
    if target == OperationState::OutcomeUnknown {
        return Err(OperationLedgerError::OutcomeAlreadyRecorded {
            slot_id,
            recorded: OperationState::OutcomeUnknown,
        });
    }
    Ok(target)
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
        OperationLedgerError::OperationMismatch { .. } => "operation_mismatch",
        OperationLedgerError::SlotUnprepared { .. } => "slot_unprepared",
        OperationLedgerError::StaleFence { .. } => "stale_fence",
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

    fn slot() -> EffectSlotId {
        EffectSlotId::from_storage_bytes([0x11; 16])
    }

    fn record(state: OperationState, generation: u64) -> OperationRecord {
        compose_record(
            slot(),
            OperationId::from_storage_bytes([0x22; 16]),
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
                destination_from_text(destination_text(destination)),
                Some(destination)
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
        assert_eq!(destination_from_text("best_effort"), None);
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
        };

        assert_eq!(
            decide_prepare(slot(), &stored, &binding),
            Err(OperationLedgerError::OperationMismatch { slot_id: slot() })
        );
    }

    #[test]
    fn a_superseded_attempt_cannot_decide_the_current_one() {
        let stored = record(OperationState::Prepared, 5);
        assert_eq!(
            decide_commit(
                slot(),
                &stored,
                AttemptGeneration::new(4),
                KnownOutcome::Succeeded
            ),
            Err(OperationLedgerError::StaleFence {
                slot_id: slot(),
                current: AttemptGeneration::new(5),
            })
        );
    }

    #[test]
    fn recommitting_the_same_outcome_is_idempotent_but_a_different_one_is_refused() {
        let stored = record(OperationState::Succeeded, 5);
        assert_eq!(
            decide_commit(
                slot(),
                &stored,
                AttemptGeneration::new(5),
                KnownOutcome::Succeeded
            ),
            Ok(CommitDecision::AlreadyRecorded),
            "a lost acknowledgement is reconciled by recommitting the same evidence"
        );
        assert_eq!(
            decide_commit(
                slot(),
                &stored,
                AttemptGeneration::new(5),
                KnownOutcome::Failed
            ),
            Err(OperationLedgerError::OutcomeAlreadyRecorded {
                slot_id: slot(),
                recorded: OperationState::Succeeded,
            }),
            "the ledger must never hold two answers for one effect"
        );
    }

    #[test]
    fn adjudication_resolves_uncertainty_and_nothing_else() {
        let unknown = record(OperationState::OutcomeUnknown, 1);
        assert_eq!(
            decide_adjudication(slot(), &unknown, KnownOutcome::Succeeded),
            Ok(OperationState::Succeeded)
        );
        assert_eq!(
            decide_adjudication(slot(), &unknown, KnownOutcome::OutcomeUnknown),
            Err(OperationLedgerError::OutcomeAlreadyRecorded {
                slot_id: slot(),
                recorded: OperationState::OutcomeUnknown,
            }),
            "adjudication must leave the record determined, not still unknown"
        );

        let determined = record(OperationState::Failed, 1);
        assert_eq!(
            decide_adjudication(slot(), &determined, KnownOutcome::Succeeded),
            Err(OperationLedgerError::OutcomeAlreadyRecorded {
                slot_id: slot(),
                recorded: OperationState::Failed,
            }),
            "adjudication never overrules an answer the system determined itself"
        );
    }
}
