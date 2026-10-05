//! Shared envelope checks; runtime retains signal and state-machine policy.
use nebula_storage_port::{
    StorageError,
    store::{ControlTurnCommit, ControlTurnCommitOutcome, ControlTurnTransition},
};

pub(crate) fn validate(commit: &ControlTurnCommit<'_>) -> Result<(), StorageError> {
    let transition = commit.transition();
    let invalid = || StorageError::Configuration("control turn envelope is invalid".into());
    i64::try_from(commit.claim().generation().get()).map_err(|_| invalid())?;
    i64::try_from(transition.expected_version()).map_err(|_| invalid())?;
    i64::try_from(transition.fence().generation()).map_err(|_| invalid())?;
    if let ControlTurnTransition::Checkpoint(batch) = transition {
        if batch.reference_transition().is_some()
            || batch.expected_version() >= i64::MAX as u64
            || batch.outbox().iter().any(|row| {
                &row.scope != batch.scope()
                    || row.execution_id != batch.execution_id()
                    || &row.id == commit.claim().row_id()
            })
            || batch
                .resume_tokens()
                .iter()
                .any(|row| &row.scope != batch.scope() || row.execution_id != batch.execution_id())
        {
            return Err(invalid());
        }
        let mut ids = std::collections::HashSet::new();
        if batch.outbox().iter().any(|row| !ids.insert(row.id)) {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) fn source(commit: &ControlTurnCommit<'_>) -> &'static str {
    match commit.command() {
        nebula_storage_port::store::ControlTurnCommand::Resume { .. } => "ControlResume",
        nebula_storage_port::store::ControlTurnCommand::Restart => "ControlRestart",
        _ => "UnsupportedControl",
    }
}

pub(crate) fn observe(result: &Result<ControlTurnCommitOutcome, StorageError>) {
    if let Ok(
        ControlTurnCommitOutcome::FencedOut {
            observation_acknowledgement,
        }
        | ControlTurnCommitOutcome::ClaimFenced {
            observation_acknowledgement,
            ..
        }
        | ControlTurnCommitOutcome::FlavorMismatch {
            observation_acknowledgement,
            ..
        }
        | ControlTurnCommitOutcome::VersionConflict {
            observation_acknowledgement,
            ..
        },
    ) = result
    {
        tracing::Span::current().record(
            "observation_acknowledgement",
            observation_acknowledgement.as_str(),
        );
    }
    tracing::Span::current().record(
        "outcome",
        match result {
            Ok(ControlTurnCommitOutcome::Accepted { .. }) => "accepted",
            Ok(ControlTurnCommitOutcome::ClaimSuperseded) => "claim_superseded",
            Ok(
                ControlTurnCommitOutcome::FencedOut { .. }
                | ControlTurnCommitOutcome::ClaimFenced { .. },
            ) => "fenced",
            Ok(ControlTurnCommitOutcome::FlavorMismatch { .. }) => "flavor-mismatch",
            Ok(ControlTurnCommitOutcome::VersionConflict { .. }) => "version_conflict",
            Ok(_) => "unsupported_outcome",
            Err(StorageError::AcknowledgementUnknown { .. }) => "acknowledgement_unknown",
            Err(_) => "error",
        },
    );
}

/// Serialize a decision derived by a verified backend transaction. Callers cannot
/// supply an observation payload to a refusal branch.
pub(crate) fn refusal_payload(
    commit: &ControlTurnCommit<'_>,
    current_generation: u64,
    reason: nebula_execution::ExecutionControlReason,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<serde_json::Value, StorageError> {
    let observation = nebula_execution::ExecutionControlObservationV1::new(
        nebula_execution::ExecutionControlSource::ControlQueue {
            row_id: *commit.claim().row_id(),
            queue_claim_generation: commit.claim().generation().get(),
        },
        current_generation,
        reason,
    );
    serde_json::to_value(nebula_execution::JournalEntry::ControlObserved {
        timestamp,
        observation,
    })
    .map_err(|_| StorageError::Internal("control refusal observation encoding failed".into()))
}

/// Derive the closed retained source kind from backend-owned marker vocabulary.
pub(crate) fn accepted_source_kind(
    source: &str,
) -> Result<(nebula_execution::ExecutionControlQueueKind, &'static str), StorageError> {
    match source {
        "Job" => Ok((
            nebula_execution::ExecutionControlQueueKind::JobDispatch,
            "job_accepted_turn",
        )),
        "ControlStart" | "ControlResume" | "ControlRestart" => Ok((
            nebula_execution::ExecutionControlQueueKind::ControlQueue,
            "control_accepted_turn",
        )),
        _ => Err(StorageError::Internal(
            "accepted turn source is invalid".into(),
        )),
    }
}

pub(crate) fn admission_payload(
    refusal: &nebula_storage_port::store::ExecutionAdmissionRefusal<'_>,
    source_kind: nebula_execution::ExecutionControlQueueKind,
    row_id: [u8; 16],
    generation: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<serde_json::Value, StorageError> {
    let observation = nebula_execution::ExecutionControlObservationV1::new(
        nebula_execution::ExecutionControlSource::AcceptedTurn {
            source_kind,
            source_row_id: row_id,
            accepted_execution_lease_generation: generation,
        },
        generation,
        nebula_execution::ExecutionControlReason::AdmissionThrottled,
    )
    .with_attempt(refusal.node_key().clone(), refusal.attempt());
    serde_json::to_value(nebula_execution::JournalEntry::ControlObserved {
        timestamp,
        observation,
    })
    .map_err(|_| StorageError::Internal("admission observation encoding failed".into()))
}

/// Encode an accepted turn inside the handoff transaction that granted its lease.
/// `generation` is the execution lease generation that transaction installed.
pub(crate) fn accepted_turn_payload(
    source: nebula_execution::ExecutionControlSource,
    generation: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<serde_json::Value, StorageError> {
    serde_json::to_value(nebula_execution::JournalEntry::ControlObserved {
        timestamp,
        observation: nebula_execution::ExecutionControlObservationV1::new(
            source,
            generation,
            nebula_execution::ExecutionControlReason::ControlAccepted,
        ),
    })
    .map_err(|_| StorageError::Internal("accepted turn observation encoding failed".into()))
}

/// Encode a recovery acceptance inside the transaction that replaced the
/// retained marker. The source keeps the original delivery and its historical
/// generation; `generation` is the new execution lease generation.
pub(crate) fn recovered_turn_payload(
    marker_source: &str,
    source_row_id: [u8; 16],
    accepted_generation: u64,
    generation: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<serde_json::Value, StorageError> {
    let (source_kind, _) = accepted_source_kind(marker_source)?;
    serde_json::to_value(nebula_execution::JournalEntry::ControlObserved {
        timestamp,
        observation: nebula_execution::ExecutionControlObservationV1::new(
            nebula_execution::ExecutionControlSource::AcceptedTurn {
                source_kind,
                source_row_id,
                accepted_execution_lease_generation: accepted_generation,
            },
            generation,
            nebula_execution::ExecutionControlReason::AcceptedTurnRecovered,
        ),
    })
    .map_err(|_| StorageError::Internal("recovery observation encoding failed".into()))
}

/// Encode a fixed owner refusal after the backend verifies the stored source row.
pub(crate) fn flavor_refusal_payload(
    request: &nebula_storage_port::store::ControlFlavorRefusal<'_>,
    generation: u64,
    reason: nebula_execution::ExecutionControlReason,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<serde_json::Value, StorageError> {
    serde_json::to_value(nebula_execution::JournalEntry::ControlObserved {
        timestamp,
        observation: nebula_execution::ExecutionControlObservationV1::new(
            nebula_execution::ExecutionControlSource::ControlQueue {
                row_id: *request.claim().row_id(),
                queue_claim_generation: request.claim().generation().get(),
            },
            generation,
            reason,
        ),
    })
    .map_err(|_| StorageError::Internal("control flavor observation encoding failed".into()))
}

/// Only these persisted command/target combinations represent supported control preflight.
pub(crate) fn supported_flavor_command(
    command: &str,
    target: Option<&nebula_storage_port::dto::ResumeTarget>,
) -> bool {
    matches!(
        (command, target),
        ("Resume", _) | ("Start" | "Restart", None)
    )
}

/// Fixed-size immutable flavor diagnostic retained in backend-owned receipts.
pub(crate) fn flavor_reason_snapshot(
    reason: &nebula_execution::ExecutionControlReason,
) -> Option<(
    nebula_core::WorkerFlavorRevisionId,
    nebula_core::WorkerFlavorRevisionId,
)> {
    if let nebula_execution::ExecutionControlReason::ExactFlavorMismatch { expected, actual } =
        reason
    {
        Some((*expected, *actual))
    } else {
        None
    }
}

/// Attach the observation acknowledgement to a decided flavor refusal. An
/// existing receipt answers with its own snapshot, or with none if unreadable.
pub(crate) fn acknowledged_flavor_outcome(
    decided: nebula_storage_port::store::ControlFlavorRefusalOutcome,
    acknowledgement: nebula_storage_port::store::ControlObservationAcknowledgement,
    recorded: Option<
        Result<
            (
                nebula_core::WorkerFlavorRevisionId,
                nebula_core::WorkerFlavorRevisionId,
            ),
            StorageError,
        >,
    >,
) -> nebula_storage_port::store::ControlFlavorRefusalOutcome {
    use nebula_storage_port::store::ControlFlavorRefusalOutcome as Flavor;
    match decided {
        Flavor::FlavorMismatch {
            snapshot, backend, ..
        } => {
            // An existing receipt answers with its own snapshot or none,
            // never with this retry's values.
            let snapshot = recorded.map_or(snapshot, recorded_flavor_snapshot);
            Flavor::FlavorMismatch {
                snapshot,
                backend,
                observation_acknowledgement: acknowledgement,
            }
        },
        Flavor::ClaimFenced {
            attempted_queue_claim_generation,
            current_queue_claim_generation,
            ..
        } => Flavor::ClaimFenced {
            attempted_queue_claim_generation,
            current_queue_claim_generation,
            observation_acknowledgement: acknowledgement,
        },
        other => other,
    }
}

/// The immutable snapshot of an existing flavor-mismatch receipt, or `None`
/// when it cannot be read. The receipt stays durable either way; the retrying
/// runtime's values are never substituted for it.
pub(crate) fn recorded_flavor_snapshot(
    read: Result<
        (
            nebula_core::WorkerFlavorRevisionId,
            nebula_core::WorkerFlavorRevisionId,
        ),
        StorageError,
    >,
) -> Option<nebula_storage_port::store::FlavorMismatchSnapshot> {
    match read {
        Ok((expected, actual)) => {
            Some(nebula_storage_port::store::FlavorMismatchSnapshot { expected, actual })
        },
        Err(error) => {
            tracing::warn!(
                %error,
                "flavor-mismatch receipt is durable but its snapshot could not be read"
            );
            None
        },
    }
}
