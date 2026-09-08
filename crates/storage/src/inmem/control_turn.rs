use nebula_storage_port::{
    StorageError, TransitionOutcome,
    store::{ControlTurnCommit, ControlTurnCommitOutcome as Outcome, ControlTurnTransition},
};

#[tracing::instrument(name = "control_turn.commit", skip_all, fields(backend = "in_memory", outcome = tracing::field::Empty))]
pub(super) fn commit(
    inner: &super::execution::SharedState,
    clock: &dyn nebula_core::accessor::Clock,
    commit: &ControlTurnCommit<'_>,
) -> Result<Outcome, StorageError> {
    let result = (|| {
        crate::control_turn::validate(commit)?;
        let transition = commit.transition();
        let scope = transition.scope();
        let id = transition.execution_id();
        let mut state = inner.lock();
        let matches = state.queue.get(commit.claim().row_id()).is_some_and(|row| {
            row.status == "Processing"
                && row.claim_generation == commit.claim().generation().get()
                && &row.msg.scope == scope
                && row.msg.execution_id == id
                && row.msg.command.as_str() == commit.command().as_str()
                && row.msg.resume_target.as_ref() == commit.command().target()
        });
        if !matches {
            return Ok(Outcome::ClaimSuperseded);
        }
        let Some(row) = state.rows.get(id).filter(|row| &row.scope == scope) else {
            return Ok(Outcome::ClaimSuperseded);
        };
        let Ok(execution) = id.parse() else {
            return Ok(Outcome::ClaimSuperseded);
        };
        if !super::plan_flavor_catalog::execution_matches_live_flavor(
            &state.revision_catalog,
            execution,
            commit.worker_flavor_revision_id(),
        ) {
            return Ok(Outcome::ClaimSuperseded);
        }
        let fence = transition.fence();
        if fence.generation() == 0
            || row.fencing_generation != fence.generation()
            || row.lease_holder.is_none()
            || row
                .lease_expires_at
                .is_none_or(|expiry| expiry < clock.now())
        {
            return Ok(Outcome::FencedOut);
        }
        if row.version != transition.expected_version() {
            return Ok(Outcome::VersionConflict {
                actual: row.version,
            });
        }
        if let ControlTurnTransition::Checkpoint(batch) = transition
            && batch
                .outbox()
                .iter()
                .any(|row| state.queue.contains_key(&row.id))
        {
            return Err(StorageError::Configuration(
                "control turn outbox collision".into(),
            ));
        }
        let new_version = match transition {
            ControlTurnTransition::Unchanged {
                expected_version, ..
            } => *expected_version,
            ControlTurnTransition::Checkpoint(batch) => {
                match super::execution::commit_locked(&mut state, batch)? {
                    TransitionOutcome::Applied { new_version } => new_version,
                    TransitionOutcome::FencedOut => return Ok(Outcome::FencedOut),
                    TransitionOutcome::VersionConflict { actual } => {
                        return Ok(Outcome::VersionConflict { actual });
                    },
                }
            },
            _ => {
                return Err(StorageError::Configuration(
                    "unsupported control turn transition".into(),
                ));
            },
        };
        // Queue presence was checked under this same lock; checkpoint envelopes
        // cannot replace the accepted command. No fallible work remains.
        if let Some(row) = state.queue.get_mut(commit.claim().row_id()) {
            row.status = "Completed".into();
            row.error_message = None;
        }
        state.accepted_turns.insert(
            id.to_owned(),
            super::turn_recovery::AcceptedTurn {
                scope: scope.clone(),
                generation: fence.generation(),
                source: crate::control_turn::source(commit),
                queue_id: *commit.claim().row_id(),
            },
        );
        Ok(Outcome::Accepted { fence, new_version })
    })();
    crate::control_turn::observe(&result);
    result
}
