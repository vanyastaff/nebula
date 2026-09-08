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
    tracing::Span::current().record(
        "outcome",
        match result {
            Ok(ControlTurnCommitOutcome::Accepted { .. }) => "accepted",
            Ok(ControlTurnCommitOutcome::ClaimSuperseded) => "claim_superseded",
            Ok(ControlTurnCommitOutcome::FencedOut) => "fenced_out",
            Ok(ControlTurnCommitOutcome::VersionConflict { .. }) => "version_conflict",
            Ok(_) => "unsupported_outcome",
            Err(StorageError::AcknowledgementUnknown { .. }) => "acknowledgement_unknown",
            Err(_) => "error",
        },
    );
}
