use nebula_storage_port::{
    StorageError, TransitionOutcome,
    store::{
        ControlObservationAcknowledgement as Ack, ControlTurnCommit,
        ControlTurnCommitOutcome as Outcome, ControlTurnTransition,
    },
};
use sqlx::{PgPool, Row};

fn backend_error(_: sqlx::Error) -> StorageError {
    StorageError::Connection("control turn backend unavailable".into())
}

#[tracing::instrument(name = "control_turn.commit", skip_all, fields(backend = "postgres", outcome = tracing::field::Empty, observation_acknowledgement = tracing::field::Empty))]
pub(super) async fn commit(
    pool: &PgPool,
    commit: &ControlTurnCommit<'_>,
) -> Result<Outcome, StorageError> {
    let result = async {
        crate::control_turn::validate(commit)?;
        let transition = commit.transition();
        let scope = transition.scope();
        let id = transition.execution_id();
        let mut tx = pool.begin().await.map_err(backend_error)?;
        // Aggregate first, then command: the same lock order as Start acceptance.
        let Some(row) = sqlx::query("SELECT version, fencing_generation, lease_holder, lease_expires_at_ms FROM port_executions WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE")
            .bind(id).bind(&scope.workspace_id).bind(&scope.org_id)
            .fetch_optional(&mut *tx).await.map_err(backend_error)? else {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::ClaimSuperseded);
        };
        let claim_generation = i64::try_from(commit.claim().generation().get())
            .map_err(|_| StorageError::Configuration("control claim generation is invalid".into()))?;
        let Some(command) = sqlx::query("SELECT command, resume_target, claim_generation FROM port_control_queue WHERE id = $1 AND execution_id = $2 AND workspace_id = $3 AND org_id = $4 AND status = 'Processing' FOR UPDATE")
            .bind(commit.claim().row_id().as_slice()).bind(id).bind(&scope.workspace_id).bind(&scope.org_id)
            .fetch_optional(&mut *tx).await.map_err(backend_error)? else {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::ClaimSuperseded);
        };
        let stored_command: String = command.try_get("command").map_err(backend_error)?;
        let encoded_target: Option<String> = command.try_get("resume_target").map_err(backend_error)?;
        let target: Option<nebula_storage_port::dto::ResumeTarget> = encoded_target.as_deref()
            .map(serde_json::from_str).transpose()
            .map_err(|_| StorageError::Internal("stored control target is invalid".into()))?;
        if stored_command != commit.command().as_str() || target.as_ref() != commit.command().target() {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::ClaimSuperseded);
        }
        let generation: i64 = row.try_get("fencing_generation").map_err(backend_error)?;
        let current_generation = u64::try_from(generation)
            .map_err(|_| StorageError::Internal("control turn generation is invalid".into()))?;
        let current_claim = u64::try_from(command.try_get::<i64, _>("claim_generation").map_err(backend_error)?)
            .map_err(|_| StorageError::Internal("control claim stored generation is invalid".into()))?;
        let attempted_claim = commit.claim().generation().get();
        if current_claim != attempted_claim {
            let observation_acknowledgement = finish_refusal(tx, commit, current_generation,
                nebula_execution::ExecutionControlReason::ClaimSuperseded {
                    attempted_queue_claim_generation: attempted_claim,
                    current_queue_claim_generation: current_claim,
                }).await;
            return Ok(Outcome::ClaimFenced {
                attempted_queue_claim_generation: attempted_claim,
                current_queue_claim_generation: current_claim,
                observation_acknowledgement,
            });
        }
        let expected: Option<Vec<u8>> = sqlx::query_scalar("SELECT worker_flavor_id FROM port_execution_revision_refs WHERE execution_id = $1 AND reference_state = 'live'")
            .bind(id).fetch_optional(&mut *tx).await.map_err(backend_error)?;
        let Some(expected) = expected else {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::ClaimSuperseded);
        };
        let expected = nebula_core::WorkerFlavorRevisionId::from_bytes(
            expected.try_into().map_err(|_| StorageError::Internal("control turn flavor is invalid".into()))?
        );
        let actual = commit.worker_flavor_revision_id();
        if expected != actual {
            // The mismatch is decided; its observation can only follow it. An
            // existing receipt reports its own immutable snapshot, or none.
            let decided = nebula_storage_port::store::FlavorMismatchSnapshot { expected, actual };
            let (snapshot, observation_acknowledgement) = match record_refusal(&mut tx, commit, current_generation,
                nebula_execution::ExecutionControlReason::ExactFlavorMismatch { expected, actual }).await {
                Ok(Ack::AlreadyRecorded) => {
                    let recorded = crate::control_turn::recorded_flavor_snapshot(
                        read_flavor_receipt(&mut tx, scope, id, commit.claim().row_id(), claim_generation).await,
                    );
                    (recorded, commit_observation(tx, Ack::AlreadyRecorded).await)
                },
                Ok(acknowledgement) => (Some(decided), commit_observation(tx, acknowledgement).await),
                Err(_) => {
                    drop(tx.rollback().await);
                    (Some(decided), Ack::Unrecorded)
                },
            };
            return Ok(Outcome::FlavorMismatch { snapshot, observation_acknowledgement });
        }
        let holder: Option<String> = row.try_get("lease_holder").map_err(backend_error)?;
        let expiry: Option<i64> = row.try_get("lease_expires_at_ms").map_err(backend_error)?;
        let now: i64 = sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(&mut *tx).await.map_err(backend_error)?;
        let fence = transition.fence();
        if generation <= 0 || u64::try_from(generation).ok() != Some(fence.generation())
            || holder.is_none() || expiry.is_none_or(|expiry| expiry < now) {
            let observation_acknowledgement = finish_refusal(tx, commit, current_generation,
                if current_generation != fence.generation() {
                    nebula_execution::ExecutionControlReason::LeaseFenced {
                        attempted_execution_lease_generation: fence.generation(),
                        current_execution_lease_generation: current_generation,
                    }
                } else if holder.is_none() {
                    nebula_execution::ExecutionControlReason::LeaseAbsent {
                        attempted_execution_lease_generation: fence.generation(),
                        current_execution_lease_generation: current_generation,
                    }
                } else {
                    nebula_execution::ExecutionControlReason::LeaseExpired {
                        attempted_execution_lease_generation: fence.generation(),
                        current_execution_lease_generation: current_generation,
                    }
                }).await;
            return Ok(Outcome::FencedOut { observation_acknowledgement });
        }
        let version = u64::try_from(row.try_get::<i64, _>("version").map_err(backend_error)?)
            .map_err(|_| StorageError::Internal("control turn stored version is invalid".into()))?;
        if version != transition.expected_version() {
            let observation_acknowledgement = finish_refusal(tx, commit, current_generation,
                nebula_execution::ExecutionControlReason::ExecutionVersionConflict {
                    expected_version: transition.expected_version(), actual_version: version,
                }).await;
            return Ok(Outcome::VersionConflict { actual: version, observation_acknowledgement });
        }
        let new_version = match transition {
            ControlTurnTransition::Unchanged { .. } => version,
            ControlTurnTransition::Checkpoint(batch) => match super::execution::commit_locked(&mut tx, batch).await? {
                TransitionOutcome::Applied { new_version } => new_version,
                TransitionOutcome::FencedOut | TransitionOutcome::VersionConflict { .. } => {
                    tx.rollback().await.map_err(backend_error)?;
                    return Err(StorageError::Internal("verified control turn changed inside owner transaction".into()));
                },
            },
            _ => {
                tx.rollback().await.map_err(backend_error)?;
                return Err(StorageError::Configuration(
                    "unsupported control turn transition".into(),
                ));
            },
        };
        let timestamp = observation_timestamp(&mut tx).await?;
        let payload = crate::control_turn::refusal_payload(commit, current_generation,
            nebula_execution::ExecutionControlReason::ControlAccepted, timestamp)?;
        append_observation(&mut tx, id, &payload).await?;
        let accepted = sqlx::query("INSERT INTO port_execution_turn_acceptances (execution_id, workspace_id, org_id, last_accepted_fencing_generation, source_kind, source_queue_id) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT(execution_id) DO UPDATE SET last_accepted_fencing_generation = excluded.last_accepted_fencing_generation, source_kind = excluded.source_kind, source_queue_id = excluded.source_queue_id WHERE port_execution_turn_acceptances.workspace_id = excluded.workspace_id AND port_execution_turn_acceptances.org_id = excluded.org_id")
            .bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind(generation)
            .bind(crate::control_turn::source(commit)).bind(commit.claim().row_id().as_slice())
            .execute(&mut *tx).await.map_err(backend_error)?;
        if accepted.rows_affected() != 1 {
            return Err(StorageError::Internal("control turn scope changed".into()));
        }
        let completed = sqlx::query("UPDATE port_control_queue SET status = 'Completed', error_message = NULL WHERE id = $1 AND execution_id = $2 AND workspace_id = $3 AND org_id = $4 AND claim_generation = $5 AND status = 'Processing'")
            .bind(commit.claim().row_id().as_slice()).bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind(claim_generation)
            .execute(&mut *tx).await.map_err(backend_error)?;
        if completed.rows_affected() != 1 {
            return Err(StorageError::Internal("locked control claim changed".into()));
        }
        tx.commit().await.map_err(|_| StorageError::AcknowledgementUnknown { operation: "control_turn_commit" })?;
        Ok(Outcome::Accepted { fence, new_version })
    }.await;
    crate::control_turn::observe(&result);
    result
}

/// Record a decided refusal's observation and commit it, never replacing the
/// decision. A refusal leaves the aggregate, marker and queue untouched, so the
/// caller's outcome is definite whatever happens to its observation: a failed
/// receipt or journal write rolls back to [`Ack::Unrecorded`], and a commit
/// whose acknowledgement is lost reports [`Ack::Unknown`].
async fn finish_refusal(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    commit: &ControlTurnCommit<'_>,
    generation: u64,
    reason: nebula_execution::ExecutionControlReason,
) -> Ack {
    match record_refusal(&mut tx, commit, generation, reason).await {
        Ok(acknowledgement) => commit_observation(tx, acknowledgement).await,
        Err(error) => {
            tracing::warn!(%error, "control refusal observation could not be written");
            drop(tx.rollback().await);
            Ack::Unrecorded
        },
    }
}

/// Commit a refusal's observation; a lost acknowledgement leaves only the
/// observation, never the refusal, in doubt.
async fn commit_observation(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    acknowledgement: Ack,
) -> Ack {
    match tx.commit().await {
        Ok(()) => acknowledgement,
        // An existing receipt wrote nothing here and stays durable.
        Err(_) if acknowledgement == Ack::AlreadyRecorded => acknowledgement,
        Err(error) => {
            tracing::warn!(%error, "control refusal observation commit was not acknowledged");
            Ack::Unknown
        },
    }
}

/// Refusal receipt and journal append share the already-held aggregate/claim lock.
async fn record_refusal(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    commit: &ControlTurnCommit<'_>,
    generation: u64,
    reason: nebula_execution::ExecutionControlReason,
) -> Result<Ack, StorageError> {
    let transition = commit.transition();
    let scope = transition.scope();
    let id = transition.execution_id();
    let claim_generation = i64::try_from(commit.claim().generation().get())
        .map_err(|_| StorageError::Configuration("control claim generation is invalid".into()))?;
    let outcome = reason.outcome().as_str();
    let snapshot = crate::control_turn::flavor_reason_snapshot(&reason);
    let payload = crate::control_turn::refusal_payload(
        commit,
        generation,
        reason,
        observation_timestamp(tx).await?,
    )?;
    let inserted = sqlx::query("INSERT INTO port_execution_control_observation_receipts (execution_id, workspace_id, org_id, source_kind, source_queue_id, source_generation, decision_key, outcome, expected_flavor_id, actual_flavor_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT (execution_id, workspace_id, org_id, source_kind, source_queue_id, source_generation, decision_key, outcome) DO NOTHING")
        .bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind("control_queue")
        .bind(commit.claim().row_id().as_slice()).bind(claim_generation).bind("").bind(outcome)
        .bind(snapshot.map(|(expected,_)| expected.as_bytes().to_vec()))
        .bind(snapshot.map(|(_,actual)| actual.as_bytes().to_vec()))
        .execute(&mut **tx).await.map_err(backend_error)?;
    if inserted.rows_affected() == 0 {
        return Ok(Ack::AlreadyRecorded);
    }
    append_observation(tx, id, &payload).await?;
    Ok(Ack::Recorded)
}

#[tracing::instrument(
    name = "execution.admission_refusal",
    skip_all,
    fields(backend = "postgres")
)]
pub(super) async fn record_admission(
    pool: &PgPool,
    refusal: &nebula_storage_port::store::ExecutionAdmissionRefusal<'_>,
) -> Result<nebula_storage_port::store::ExecutionAdmissionRefusalOutcome, StorageError> {
    use nebula_storage_port::store::ExecutionAdmissionRefusalOutcome as Admission;
    let mut tx = pool.begin().await.map_err(backend_error)?;
    let id = refusal.execution_id();
    let scope = refusal.scope();
    let Some(row) = sqlx::query("SELECT fencing_generation, lease_holder, lease_expires_at_ms FROM port_executions WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE")
        .bind(id).bind(&scope.workspace_id).bind(&scope.org_id)
        .fetch_optional(&mut *tx).await.map_err(backend_error)? else {
        return Ok(Admission::FencedOut);
    };
    let generation: i64 = row.try_get("fencing_generation").map_err(backend_error)?;
    let holder: Option<String> = row.try_get("lease_holder").map_err(backend_error)?;
    let expiry: Option<i64> = row.try_get("lease_expires_at_ms").map_err(backend_error)?;
    let now: i64 =
        sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(backend_error)?;
    if generation <= 0
        || u64::try_from(generation).ok() != Some(refusal.fence().generation())
        || holder.is_none()
        || expiry.is_none_or(|expiry| expiry < now)
    {
        return Ok(Admission::FencedOut);
    }
    let Some(marker) = sqlx::query("SELECT source_kind, source_queue_id FROM port_execution_turn_acceptances WHERE execution_id = $1 AND workspace_id = $2 AND org_id = $3 AND last_accepted_fencing_generation = $4")
        .bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind(generation)
        .fetch_optional(&mut *tx).await.map_err(backend_error)? else {
        return Ok(Admission::MissingAcceptedTurn);
    };
    let source: String = marker.try_get("source_kind").map_err(backend_error)?;
    let row_id: Vec<u8> = marker.try_get("source_queue_id").map_err(backend_error)?;
    let row_id: [u8; 16] = row_id
        .try_into()
        .map_err(|_| StorageError::Internal("admission source identity is invalid".into()))?;
    let (source_kind, receipt_kind) = crate::control_turn::accepted_source_kind(&source)?;
    let payload = crate::control_turn::admission_payload(
        refusal,
        source_kind,
        row_id,
        refusal.fence().generation(),
        chrono::DateTime::from_timestamp_millis(now)
            .ok_or_else(|| StorageError::Internal("backend clock is invalid".into()))?,
    )?;
    let decision_key = format!(
        "node/{}/attempt/{}",
        refusal.node_key().as_str(),
        refusal.attempt()
    );
    // The owner is verified and the throttle attributed: the observation
    // below only follows that decision and never replaces it.
    let written = async {
        let inserted = sqlx::query("INSERT INTO port_execution_control_observation_receipts (execution_id, workspace_id, org_id, source_kind, source_queue_id, source_generation, decision_key, outcome, expected_flavor_id, actual_flavor_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT(execution_id, workspace_id, org_id, source_kind, source_queue_id, source_generation, decision_key, outcome) DO NOTHING")
            .bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind(receipt_kind)
            .bind(row_id.as_slice()).bind(generation).bind(decision_key).bind("throttled")
            .bind(Option::<Vec<u8>>::None).bind(Option::<Vec<u8>>::None)
            .execute(&mut *tx).await.map_err(backend_error)?.rows_affected() == 1;
        if inserted {
            append_observation(&mut tx, id, &payload).await?;
        }
        Ok::<bool, StorageError>(inserted)
    }
    .await;
    let observation_acknowledgement = match written {
        Ok(true) => commit_observation(tx, Ack::Recorded).await,
        Ok(false) => {
            drop(tx.rollback().await);
            Ack::AlreadyRecorded
        },
        Err(error) => {
            tracing::warn!(%error, "admission refusal observation could not be written");
            drop(tx.rollback().await);
            Ack::Unrecorded
        },
    };
    Ok(Admission::Attributed {
        backend: nebula_storage_port::StorageBackendKind::Postgres,
        observation_acknowledgement,
    })
}

/// Internal aggregate-owner append; caller already holds the scoped execution lock.
pub(super) async fn append_observation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    execution_id: &str,
    payload: &serde_json::Value,
) -> Result<(), StorageError> {
    let seq: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM port_execution_journal WHERE execution_id = $1",
    )
    .bind(execution_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(backend_error)?;
    sqlx::query(
        "INSERT INTO port_execution_journal (execution_id, seq, payload) VALUES ($1, $2, $3)",
    )
    .bind(execution_id)
    .bind(seq)
    .bind(payload)
    .execute(&mut **tx)
    .await
    .map_err(backend_error)?;
    Ok(())
}

#[tracing::instrument(
    name = "control_turn.flavor_refusal",
    skip_all,
    fields(backend = "postgres")
)]
pub(super) async fn record_flavor(
    pool: &PgPool,
    request: &nebula_storage_port::store::ControlFlavorRefusal<'_>,
) -> Result<nebula_storage_port::store::ControlFlavorRefusalOutcome, StorageError> {
    use nebula_storage_port::store::ControlFlavorRefusalOutcome as Flavor;
    let mut tx = pool.begin().await.map_err(backend_error)?;
    let scope = request.claim().scope();
    let id = request.execution_id();
    let Some(row) = sqlx::query("SELECT fencing_generation FROM port_executions WHERE id = $1 AND workspace_id = $2 AND org_id = $3 FOR UPDATE")
        .bind(id).bind(&scope.workspace_id).bind(&scope.org_id)
        .fetch_optional(&mut *tx).await.map_err(backend_error)? else { return Ok(Flavor::ClaimSuperseded); };
    let generation = u64::try_from(
        row.try_get::<i64, _>("fencing_generation")
            .map_err(backend_error)?,
    )
    .map_err(|_| StorageError::Internal("stored execution generation is invalid".into()))?;
    let Some(command) = sqlx::query("SELECT command, resume_target, claim_generation FROM port_control_queue WHERE id = $1 AND execution_id = $2 AND workspace_id = $3 AND org_id = $4 AND status = 'Processing' FOR UPDATE")
        .bind(request.claim().row_id().as_slice()).bind(id).bind(&scope.workspace_id).bind(&scope.org_id)
        .fetch_optional(&mut *tx).await.map_err(backend_error)? else { return Ok(Flavor::ClaimSuperseded); };
    let command_kind: String = command.try_get("command").map_err(backend_error)?;
    let encoded: Option<String> = command.try_get("resume_target").map_err(backend_error)?;
    let target: Option<nebula_storage_port::dto::ResumeTarget> = encoded
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| StorageError::Internal("stored control target is invalid".into()))?;
    if !crate::control_turn::supported_flavor_command(&command_kind, target.as_ref()) {
        return Ok(Flavor::ClaimSuperseded);
    }
    let current_claim = u64::try_from(
        command
            .try_get::<i64, _>("claim_generation")
            .map_err(backend_error)?,
    )
    .map_err(|_| StorageError::Internal("stored claim generation is invalid".into()))?;
    let attempted_claim = request.claim().generation().get();
    let source_generation = i64::try_from(attempted_claim)
        .map_err(|_| StorageError::Configuration("claim generation is invalid".into()))?;
    let Some(expected) = sqlx::query_scalar::<_,Vec<u8>>("SELECT worker_flavor_id FROM port_execution_revision_refs WHERE execution_id = $1 AND reference_state = 'live'")
        .bind(id).fetch_optional(&mut *tx).await.map_err(backend_error)? else { return Ok(Flavor::ClaimSuperseded); };
    let expected = nebula_core::WorkerFlavorRevisionId::from_bytes(
        expected
            .try_into()
            .map_err(|_| StorageError::Internal("stored flavor is invalid".into()))?,
    );
    let actual = request.actual_worker_flavor_revision_id();
    let backend = nebula_storage_port::StorageBackendKind::Postgres;
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
                backend,
                observation_acknowledgement: Ack::Recorded,
            },
        )
    };
    // The refusal is decided; everything below only observes it and can
    // never replace it.
    let receipt_outcome = reason.outcome().as_str();
    let snapshot = crate::control_turn::flavor_reason_snapshot(&reason);
    let written = async {
        let timestamp =
            sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>("SELECT clock_timestamp()")
                .fetch_one(&mut *tx)
                .await
                .map_err(backend_error)?;
        let payload =
            crate::control_turn::flavor_refusal_payload(request, generation, reason, timestamp)?;
        let inserted = sqlx::query("INSERT INTO port_execution_control_observation_receipts (execution_id,workspace_id,org_id,source_kind,source_queue_id,source_generation,decision_key,outcome,expected_flavor_id,actual_flavor_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) ON CONFLICT(execution_id,workspace_id,org_id,source_kind,source_queue_id,source_generation,decision_key,outcome) DO NOTHING")
            .bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind("control_queue")
            .bind(request.claim().row_id().as_slice()).bind(source_generation).bind("").bind(receipt_outcome)
            .bind(snapshot.map(|(expected,_)| expected.as_bytes().to_vec()))
            .bind(snapshot.map(|(_,actual)| actual.as_bytes().to_vec()))
            .execute(&mut *tx).await.map_err(backend_error)?.rows_affected()==1;
        if inserted {
            append_observation(&mut tx, id, &payload).await?;
        }
        Ok::<bool, StorageError>(inserted)
    }
    .await;
    let (acknowledgement, recorded_snapshot) = settle_flavor_observation(
        tx,
        written,
        snapshot.is_some(),
        scope,
        id,
        request.claim().row_id(),
        source_generation,
    )
    .await;
    Ok(crate::control_turn::acknowledged_flavor_outcome(
        outcome,
        acknowledgement,
        recorded_snapshot,
    ))
}

/// Commit or roll back a decided flavor refusal's observation. An existing
/// receipt is durable whatever happens to this transaction; a failed write
/// rolls back as unrecorded and a lost commit acknowledgement is unknown.
async fn settle_flavor_observation(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    written: Result<bool, StorageError>,
    has_snapshot: bool,
    scope: &nebula_storage_port::Scope,
    id: &str,
    row_id: &[u8; 16],
    source_generation: i64,
) -> (
    Ack,
    Option<
        Result<
            (
                nebula_core::WorkerFlavorRevisionId,
                nebula_core::WorkerFlavorRevisionId,
            ),
            StorageError,
        >,
    >,
) {
    match written {
        Ok(true) => (commit_observation(tx, Ack::Recorded).await, None),
        Ok(false) => {
            let recorded = if has_snapshot {
                Some(read_flavor_receipt(&mut tx, scope, id, row_id, source_generation).await)
            } else {
                None
            };
            drop(tx.rollback().await);
            (Ack::AlreadyRecorded, recorded)
        },
        Err(error) => {
            tracing::warn!(%error, "control flavor refusal observation could not be written");
            drop(tx.rollback().await);
            (Ack::Unrecorded, None)
        },
    }
}

async fn observation_timestamp(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<chrono::DateTime<chrono::Utc>, StorageError> {
    let milliseconds: i64 =
        sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(&mut **tx)
            .await
            .map_err(backend_error)?;
    chrono::DateTime::from_timestamp_millis(milliseconds)
        .ok_or_else(|| StorageError::Internal("backend clock is invalid".into()))
}

async fn read_flavor_receipt(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &nebula_storage_port::Scope,
    id: &str,
    row_id: &[u8; 16],
    claim_generation: i64,
) -> Result<
    (
        nebula_core::WorkerFlavorRevisionId,
        nebula_core::WorkerFlavorRevisionId,
    ),
    StorageError,
> {
    let receipt = sqlx::query("SELECT expected_flavor_id, actual_flavor_id FROM port_execution_control_observation_receipts WHERE execution_id = $1 AND workspace_id = $2 AND org_id = $3 AND source_kind = 'control_queue' AND source_queue_id = $4 AND source_generation = $5 AND decision_key = '' AND outcome = 'flavor-mismatch'")
        .bind(id).bind(&scope.workspace_id).bind(&scope.org_id).bind(row_id.as_slice()).bind(claim_generation)
        .fetch_one(&mut **tx).await.map_err(backend_error)?;
    let expected: Vec<u8> = receipt
        .try_get("expected_flavor_id")
        .map_err(backend_error)?;
    let actual: Vec<u8> = receipt.try_get("actual_flavor_id").map_err(backend_error)?;
    let expected = nebula_core::WorkerFlavorRevisionId::from_bytes(
        expected
            .try_into()
            .map_err(|_| StorageError::Internal("flavor receipt snapshot is invalid".into()))?,
    );
    let actual = nebula_core::WorkerFlavorRevisionId::from_bytes(
        actual
            .try_into()
            .map_err(|_| StorageError::Internal("flavor receipt snapshot is invalid".into()))?,
    );
    Ok((expected, actual))
}
