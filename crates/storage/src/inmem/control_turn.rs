use nebula_storage_port::{
    StorageError, TransitionOutcome,
    store::{
        ControlObservationAcknowledgement as Ack, ControlTurnCommit,
        ControlTurnCommitOutcome as Outcome, ControlTurnTransition,
    },
};

#[tracing::instrument(name = "control_turn.commit", skip_all, fields(backend = "in_memory", outcome = tracing::field::Empty, observation_acknowledgement = tracing::field::Empty))]
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
        let identity = id.to_owned();
        let mut state = inner.lock();
        let matches = state.queue.get(commit.claim().row_id()).is_some_and(|row| {
            row.status == "Processing"
                && &row.msg.scope == scope
                && row.msg.execution_id == id
                && row.msg.command.as_str() == commit.command().as_str()
                && row.msg.resume_target.as_ref() == commit.command().target()
        });
        if !matches {
            return Ok(Outcome::ClaimSuperseded);
        }
        let Some(row) = state.rows.get(&identity).filter(|row| &row.scope == scope) else {
            return Ok(Outcome::ClaimSuperseded);
        };
        let Ok(execution) = id.parse() else {
            return Ok(Outcome::ClaimSuperseded);
        };
        let current_generation = row.fencing_generation;
        let current_claim = state
            .queue
            .get(commit.claim().row_id())
            .map(|row| row.claim_generation)
            .ok_or_else(|| StorageError::Internal("verified control claim disappeared".into()))?;
        let attempted_claim = commit.claim().generation().get();
        if current_claim != attempted_claim {
            let observation_acknowledgement = record_refusal(
                &mut state,
                commit,
                current_generation,
                nebula_execution::ExecutionControlReason::ClaimSuperseded {
                    attempted_queue_claim_generation: attempted_claim,
                    current_queue_claim_generation: current_claim,
                },
                clock.now(),
            );
            return Ok(Outcome::ClaimFenced {
                attempted_queue_claim_generation: attempted_claim,
                current_queue_claim_generation: current_claim,
                observation_acknowledgement,
            });
        }
        let Some(expected) =
            super::plan_flavor_catalog::execution_live_flavor(&state.revision_catalog, execution)
        else {
            return Ok(Outcome::ClaimSuperseded);
        };
        let actual = commit.worker_flavor_revision_id();
        if expected != actual {
            let observation_acknowledgement = record_refusal(
                &mut state,
                commit,
                current_generation,
                nebula_execution::ExecutionControlReason::ExactFlavorMismatch { expected, actual },
                clock.now(),
            );
            let snapshot = if observation_acknowledgement == Ack::AlreadyRecorded {
                crate::control_turn::recorded_flavor_snapshot(read_flavor_receipt(
                    &state,
                    &identity,
                    commit.claim().row_id(),
                    commit.claim().generation().get(),
                ))
            } else {
                Some(nebula_storage_port::store::FlavorMismatchSnapshot { expected, actual })
            };
            return Ok(Outcome::FlavorMismatch {
                snapshot,
                observation_acknowledgement,
            });
        }
        let fence = transition.fence();
        if fence.generation() == 0
            || row.fencing_generation != fence.generation()
            || row.lease_holder.is_none()
            || row
                .lease_expires_at
                .is_none_or(|expiry| expiry < clock.now())
        {
            let reason = if current_generation != fence.generation() {
                nebula_execution::ExecutionControlReason::LeaseFenced {
                    attempted_execution_lease_generation: fence.generation(),
                    current_execution_lease_generation: current_generation,
                }
            } else if row.lease_holder.is_none() {
                nebula_execution::ExecutionControlReason::LeaseAbsent {
                    attempted_execution_lease_generation: fence.generation(),
                    current_execution_lease_generation: current_generation,
                }
            } else {
                nebula_execution::ExecutionControlReason::LeaseExpired {
                    attempted_execution_lease_generation: fence.generation(),
                    current_execution_lease_generation: current_generation,
                }
            };
            let observation_acknowledgement =
                record_refusal(&mut state, commit, current_generation, reason, clock.now());
            return Ok(Outcome::FencedOut {
                observation_acknowledgement,
            });
        }
        if row.version != transition.expected_version() {
            let actual = row.version;
            let observation_acknowledgement = record_refusal(
                &mut state,
                commit,
                current_generation,
                nebula_execution::ExecutionControlReason::ExecutionVersionConflict {
                    expected_version: transition.expected_version(),
                    actual_version: actual,
                },
                clock.now(),
            );
            return Ok(Outcome::VersionConflict {
                actual,
                observation_acknowledgement,
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
        let payload = crate::control_turn::refusal_payload(
            commit,
            current_generation,
            nebula_execution::ExecutionControlReason::ControlAccepted,
            clock.now(),
        )?;
        let extra = match transition {
            ControlTurnTransition::Checkpoint(batch) => batch.journal().len(),
            _ => 0,
        };
        let increment = u64::try_from(extra)
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| StorageError::Internal("control journal sequence exhausted".into()))?;
        state
            .next_seq
            .get(&identity)
            .copied()
            .unwrap_or(1)
            .checked_add(increment)
            .ok_or_else(|| StorageError::Internal("control journal sequence exhausted".into()))?;
        let new_version = match transition {
            ControlTurnTransition::Unchanged {
                expected_version, ..
            } => {
                append_observation(&mut state, &identity, payload)?;
                *expected_version
            },
            ControlTurnTransition::Checkpoint(batch) => {
                let mut journal = batch.journal().to_vec();
                journal.push(nebula_storage_port::dto::JournalEntry { seq: None, payload });
                let observed = nebula_storage_port::TransitionBatch::builder()
                    .scope(scope.clone())
                    .execution_id(id)
                    .expected_version(batch.expected_version())
                    .fencing(batch.fencing())
                    .new_state(batch.new_state().clone())
                    .outbox(batch.outbox().to_vec())
                    .resume_tokens(batch.resume_tokens().to_vec())
                    .journal(journal)
                    .build()?;
                match super::execution::commit_locked(&mut state, &observed)? {
                    TransitionOutcome::Applied { new_version } => new_version,
                    TransitionOutcome::FencedOut | TransitionOutcome::VersionConflict { .. } => {
                        return Err(StorageError::Internal(
                            "verified control turn changed inside owner lock".into(),
                        ));
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
            identity,
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

/// Only called after the stored queue claim, target, tenant and execution were
/// verified under the same lock. Nothing from the refused batch is persisted.
///
/// The refusal is already decided; a failed observation write reports
/// [`Ack::Unrecorded`] and never replaces the decision.
fn record_refusal(
    state: &mut super::execution::State,
    commit: &ControlTurnCommit<'_>,
    generation: u64,
    reason: nebula_execution::ExecutionControlReason,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Ack {
    match try_record_refusal(state, commit, generation, reason, timestamp) {
        Ok(acknowledgement) => acknowledgement,
        Err(error) => {
            tracing::warn!(%error, "control refusal observation could not be written");
            Ack::Unrecorded
        },
    }
}

fn try_record_refusal(
    state: &mut super::execution::State,
    commit: &ControlTurnCommit<'_>,
    generation: u64,
    reason: nebula_execution::ExecutionControlReason,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Result<Ack, StorageError> {
    let identity = commit.transition().execution_id().to_owned();
    let key = (
        identity.clone(),
        "control_queue",
        *commit.claim().row_id(),
        commit.claim().generation().get(),
        String::new(),
        reason.outcome(),
    );
    if state.control_observation_receipts.contains_key(&key) {
        return Ok(Ack::AlreadyRecorded);
    }
    let snapshot = crate::control_turn::flavor_reason_snapshot(&reason);
    let payload = crate::control_turn::refusal_payload(commit, generation, reason, timestamp)?;
    append_observation(state, &identity, payload)?;
    state.control_observation_receipts.insert(key, snapshot);
    Ok(Ack::Recorded)
}

#[tracing::instrument(
    name = "execution.admission_refusal",
    skip_all,
    fields(backend = "in-memory")
)]
pub(super) fn record_admission(
    inner: &super::execution::SharedState,
    clock: &dyn nebula_core::accessor::Clock,
    refusal: &nebula_storage_port::store::ExecutionAdmissionRefusal<'_>,
) -> Result<nebula_storage_port::store::ExecutionAdmissionRefusalOutcome, StorageError> {
    use nebula_storage_port::store::ExecutionAdmissionRefusalOutcome as Admission;
    let mut state = inner.lock();
    let identity = refusal.execution_id().to_owned();
    let Some(row) = state
        .rows
        .get(&identity)
        .filter(|row| &row.scope == refusal.scope())
    else {
        return Ok(Admission::FencedOut);
    };
    if row.fencing_generation != refusal.fence().generation()
        || row.lease_holder.is_none()
        || row
            .lease_expires_at
            .is_none_or(|expiry| expiry < clock.now())
    {
        return Ok(Admission::FencedOut);
    }
    let Some(marker) = state.accepted_turns.get(&identity).filter(|marker| {
        &marker.scope == refusal.scope() && marker.generation == refusal.fence().generation()
    }) else {
        return Ok(Admission::MissingAcceptedTurn);
    };
    let (source_kind, receipt_kind) = crate::control_turn::accepted_source_kind(marker.source)?;
    let generation = marker.generation;
    let row_id = marker.queue_id;
    let decision_key = format!(
        "node/{}/attempt/{}",
        refusal.node_key().as_str(),
        refusal.attempt()
    );
    let key = (
        identity.clone(),
        receipt_kind,
        row_id,
        generation,
        decision_key,
        nebula_execution::ExecutionControlOutcome::Throttled,
    );
    let backend = nebula_storage_port::StorageBackendKind::InMemory;
    // The owner is verified and the throttle attributed: the observation
    // below only follows that decision and never replaces it.
    let observation_acknowledgement = if state.control_observation_receipts.contains_key(&key) {
        Ack::AlreadyRecorded
    } else {
        match crate::control_turn::admission_payload(
            refusal,
            source_kind,
            row_id,
            generation,
            clock.now(),
        )
        .and_then(|payload| append_observation(&mut state, &identity, payload))
        {
            Ok(()) => {
                state.control_observation_receipts.insert(key, None);
                Ack::Recorded
            },
            Err(error) => {
                tracing::warn!(%error, "admission refusal observation could not be written");
                Ack::Unrecorded
            },
        }
    };
    Ok(Admission::Attributed {
        backend,
        observation_acknowledgement,
    })
}

/// Internal aggregate-owner append; caller holds the execution state lock.
pub(super) fn append_observation(
    state: &mut super::execution::State,
    identity: &str,
    payload: serde_json::Value,
) -> Result<(), StorageError> {
    let seq = state.next_seq.get(identity).copied().unwrap_or(1);
    let next = seq.checked_add(1).ok_or_else(|| {
        StorageError::Internal("control observation journal sequence exhausted".into())
    })?;
    let row = state.rows.get_mut(identity).ok_or_else(|| {
        StorageError::Internal("control observation execution disappeared".into())
    })?;
    row.journal.push((seq, payload));
    state.next_seq.insert(identity.to_owned(), next);
    Ok(())
}

#[tracing::instrument(
    name = "control_turn.flavor_refusal",
    skip_all,
    fields(backend = "in-memory")
)]
pub(super) fn record_flavor(
    inner: &super::execution::SharedState,
    clock: &dyn nebula_core::accessor::Clock,
    request: &nebula_storage_port::store::ControlFlavorRefusal<'_>,
) -> Result<nebula_storage_port::store::ControlFlavorRefusalOutcome, StorageError> {
    use nebula_storage_port::store::ControlFlavorRefusalOutcome as Flavor;
    let scope = request.claim().scope();
    let id = request.execution_id();
    let identity = id.to_owned();
    let mut state = inner.lock();
    let Some(row) = state.rows.get(&identity).filter(|row| &row.scope == scope) else {
        return Ok(Flavor::ClaimSuperseded);
    };
    let generation = row.fencing_generation;
    let Some(command) = state.queue.get(request.claim().row_id()).filter(|row| {
        row.status == "Processing"
            && &row.msg.scope == scope
            && row.msg.execution_id == id
            && crate::control_turn::supported_flavor_command(
                row.msg.command.as_str(),
                row.msg.resume_target.as_ref(),
            )
    }) else {
        return Ok(Flavor::ClaimSuperseded);
    };
    let current_claim = command.claim_generation;
    let attempted_claim = request.claim().generation().get();
    let execution = id
        .parse()
        .map_err(|_| StorageError::Internal("stored execution identity is invalid".into()))?;
    let Some(expected) =
        super::plan_flavor_catalog::execution_live_flavor(&state.revision_catalog, execution)
    else {
        return Ok(Flavor::ClaimSuperseded);
    };
    let actual = request.actual_worker_flavor_revision_id();
    let (reason, outcome) = if current_claim != attempted_claim {
        (
            nebula_execution::ExecutionControlReason::ClaimSuperseded {
                attempted_queue_claim_generation: attempted_claim,
                current_queue_claim_generation: current_claim,
            },
            Flavor::ClaimFenced {
                attempted_queue_claim_generation: attempted_claim,
                current_queue_claim_generation: current_claim,
                observation_acknowledgement: Ack::Recorded,
            },
        )
    } else if expected == actual {
        return Ok(Flavor::NoMismatch);
    } else {
        (
            nebula_execution::ExecutionControlReason::ExactFlavorMismatch { expected, actual },
            Flavor::FlavorMismatch {
                snapshot: Some(nebula_storage_port::store::FlavorMismatchSnapshot {
                    expected,
                    actual,
                }),
                backend: nebula_storage_port::StorageBackendKind::InMemory,
                observation_acknowledgement: Ack::Recorded,
            },
        )
    };
    let key = (
        identity.clone(),
        "control_queue",
        *request.claim().row_id(),
        attempted_claim,
        String::new(),
        reason.outcome(),
    );
    // The refusal is decided; everything below only observes it.
    if state.control_observation_receipts.contains_key(&key) {
        let recorded = matches!(outcome, Flavor::FlavorMismatch { .. }).then(|| {
            read_flavor_receipt(&state, &identity, request.claim().row_id(), attempted_claim)
        });
        return Ok(crate::control_turn::acknowledged_flavor_outcome(
            outcome,
            Ack::AlreadyRecorded,
            recorded,
        ));
    }
    let snapshot = crate::control_turn::flavor_reason_snapshot(&reason);
    let acknowledgement =
        match crate::control_turn::flavor_refusal_payload(request, generation, reason, clock.now())
            .and_then(|payload| append_observation(&mut state, &identity, payload))
        {
            Ok(()) => {
                state.control_observation_receipts.insert(key, snapshot);
                Ack::Recorded
            },
            Err(error) => {
                tracing::warn!(%error, "control flavor refusal observation could not be written");
                Ack::Unrecorded
            },
        };
    Ok(crate::control_turn::acknowledged_flavor_outcome(
        outcome,
        acknowledgement,
        None,
    ))
}

fn read_flavor_receipt(
    state: &super::execution::State,
    identity: &str,
    row_id: &[u8; 16],
    claim_generation: u64,
) -> Result<
    (
        nebula_core::WorkerFlavorRevisionId,
        nebula_core::WorkerFlavorRevisionId,
    ),
    StorageError,
> {
    let key = (
        identity.to_owned(),
        "control_queue",
        *row_id,
        claim_generation,
        String::new(),
        nebula_execution::ExecutionControlOutcome::FlavorMismatch,
    );
    state
        .control_observation_receipts
        .get(&key)
        .copied()
        .flatten()
        .ok_or_else(|| StorageError::Internal("flavor receipt snapshot is missing".into()))
}
