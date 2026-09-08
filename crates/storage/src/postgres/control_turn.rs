use nebula_storage_port::{
    StorageError, TransitionOutcome,
    store::{ControlTurnCommit, ControlTurnCommitOutcome as Outcome, ControlTurnTransition},
};
use sqlx::{PgPool, Row};

fn backend_error(_: sqlx::Error) -> StorageError {
    StorageError::Connection("control turn backend unavailable".into())
}

#[tracing::instrument(name = "control_turn.commit", skip_all, fields(backend = "postgres", outcome = tracing::field::Empty))]
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
        let Some(command) = sqlx::query("SELECT command, resume_target FROM port_control_queue WHERE id = $1 AND execution_id = $2 AND workspace_id = $3 AND org_id = $4 AND status = 'Processing' AND claim_generation = $5 FOR UPDATE")
            .bind(commit.claim().row_id().as_slice()).bind(id).bind(&scope.workspace_id).bind(&scope.org_id)
            .bind(claim_generation).fetch_optional(&mut *tx).await.map_err(backend_error)? else {
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
        let references: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM port_execution_revision_refs WHERE execution_id = $1 AND worker_flavor_id = $2 AND reference_state = 'live'")
            .bind(id).bind(commit.worker_flavor_revision_id().as_bytes().as_slice())
            .fetch_one(&mut *tx).await.map_err(backend_error)?;
        if references != 1 {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::ClaimSuperseded);
        }
        let generation: i64 = row.try_get("fencing_generation").map_err(backend_error)?;
        let holder: Option<String> = row.try_get("lease_holder").map_err(backend_error)?;
        let expiry: Option<i64> = row.try_get("lease_expires_at_ms").map_err(backend_error)?;
        let now: i64 = sqlx::query_scalar("SELECT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint")
            .fetch_one(&mut *tx).await.map_err(backend_error)?;
        let fence = transition.fence();
        if generation <= 0 || u64::try_from(generation).ok() != Some(fence.generation())
            || holder.is_none() || expiry.is_none_or(|expiry| expiry < now) {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::FencedOut);
        }
        let version = u64::try_from(row.try_get::<i64, _>("version").map_err(backend_error)?)
            .map_err(|_| StorageError::Internal("control turn stored version is invalid".into()))?;
        if version != transition.expected_version() {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(Outcome::VersionConflict { actual: version });
        }
        let new_version = match transition {
            ControlTurnTransition::Unchanged { .. } => version,
            ControlTurnTransition::Checkpoint(batch) => match super::execution::commit_locked(&mut tx, batch).await? {
                TransitionOutcome::Applied { new_version } => new_version,
                TransitionOutcome::FencedOut => {
                    tx.rollback().await.map_err(backend_error)?;
                    return Ok(Outcome::FencedOut);
                },
                TransitionOutcome::VersionConflict { actual } => {
                    tx.rollback().await.map_err(backend_error)?;
                    return Ok(Outcome::VersionConflict { actual });
                },
            },
            _ => {
                tx.rollback().await.map_err(backend_error)?;
                return Err(StorageError::Configuration(
                    "unsupported control turn transition".into(),
                ));
            },
        };
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
