//! Bounded advisory discovery and atomic recovery of durably accepted turns.
use nebula_storage_port::{
    FencingToken, Scope, StorageError,
    store::{RecoverableTurn, RecoverableTurnPage, RecoveryTurnAcceptance, RecoveryTurnHandoff},
};
use sqlx::{Row, SqlitePool};

fn backend_error(_: sqlx::Error) -> StorageError {
    StorageError::Connection("execution turn recovery backend unavailable".into())
}

#[tracing::instrument(name = "turn_recovery.list", skip_all,
    fields(backend = "sqlite", limit, candidates = tracing::field::Empty))]
pub(super) async fn list(
    pool: &SqlitePool,
    flavor: nebula_core::WorkerFlavorRevisionId,
    after: Option<&str>,
    limit: u32,
) -> Result<RecoverableTurnPage, StorageError> {
    if !(1..=256).contains(&limit) {
        return Err(StorageError::Configuration(
            "recovery page limit must be in 1..=256".into(),
        ));
    }
    // Page exact live references first. Live leases still advance the cursor.
    let rows = sqlx::query("SELECT a.execution_id, a.workspace_id, a.org_id, a.last_accepted_fencing_generation, e.lease_expires_at_ms, CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER) AS scan_now FROM port_execution_turn_acceptances a JOIN port_executions e ON e.id = a.execution_id AND e.workspace_id = a.workspace_id AND e.org_id = a.org_id JOIN port_execution_revision_refs r ON r.execution_id = a.execution_id WHERE r.reference_state = 'live' AND r.worker_flavor_id = ? AND (? IS NULL OR a.execution_id > ?) ORDER BY a.execution_id LIMIT ?")
        .bind(flavor.as_bytes().as_slice()).bind(after).bind(after).bind(i64::from(limit))
        .fetch_all(pool).await.map_err(backend_error)?;
    let scanned = rows.len();
    let mut last = None;
    let mut turns = Vec::new();
    for row in rows {
        let execution_id: String = row.try_get("execution_id").map_err(backend_error)?;
        last = Some(execution_id.clone());
        let now: i64 = row.try_get("scan_now").map_err(backend_error)?;
        let expiry: Option<i64> = row.try_get("lease_expires_at_ms").map_err(backend_error)?;
        if expiry.is_none_or(|expiry| expiry < now) {
            let generation: i64 = row
                .try_get("last_accepted_fencing_generation")
                .map_err(backend_error)?;
            let generation = u64::try_from(generation)
                .ok()
                .filter(|generation| *generation > 0)
                .ok_or_else(|| {
                    StorageError::Internal("recovery marker generation is invalid".into())
                })?;
            turns.push(RecoverableTurn::new(
                Scope::new(
                    row.try_get::<String, _>("workspace_id")
                        .map_err(backend_error)?,
                    row.try_get::<String, _>("org_id").map_err(backend_error)?,
                ),
                execution_id,
                generation,
            ));
        }
    }
    tracing::Span::current().record("candidates", turns.len());
    Ok(RecoverableTurnPage::new(
        turns,
        if scanned == limit as usize {
            last
        } else {
            None
        },
    ))
}

#[tracing::instrument(name = "turn_recovery.accept", skip_all,
    fields(backend = "sqlite", execution_id = handoff.execution_id(), outcome = tracing::field::Empty))]
pub(super) async fn accept(
    pool: &SqlitePool,
    handoff: &RecoveryTurnHandoff<'_>,
) -> Result<RecoveryTurnAcceptance, StorageError> {
    let result = async {
        i64::try_from(handoff.expected_execution_version())
            .map_err(|_| StorageError::Internal("recovery execution version is invalid".into()))?;
        let expected_marker = i64::try_from(handoff.expected_accepted_fencing_generation())
            .map_err(|_| StorageError::Internal("recovery marker generation is invalid".into()))?;
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(backend_error)?;
        let Some(row) = sqlx::query("SELECT version, fencing_generation, lease_expires_at_ms FROM port_executions WHERE id = ? AND workspace_id = ? AND org_id = ?")
            .bind(handoff.execution_id()).bind(&handoff.scope().workspace_id).bind(&handoff.scope().org_id)
            .fetch_optional(&mut *tx).await.map_err(backend_error)? else {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
        };
        let marker: Option<i64> = sqlx::query_scalar("SELECT last_accepted_fencing_generation FROM port_execution_turn_acceptances a WHERE execution_id = ? AND workspace_id = ? AND org_id = ? AND EXISTS (SELECT 1 FROM port_execution_revision_refs r WHERE r.execution_id = a.execution_id AND r.reference_state = 'live' AND r.worker_flavor_id = ?)")
            .bind(handoff.execution_id()).bind(&handoff.scope().workspace_id).bind(&handoff.scope().org_id)
            .bind(handoff.worker_flavor_revision_id().as_bytes().as_slice())
            .fetch_optional(&mut *tx).await.map_err(backend_error)?;
        if marker != Some(expected_marker) {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
        }
        let version = u64::try_from(row.try_get::<i64, _>("version").map_err(backend_error)?)
            .map_err(|_| StorageError::Internal("recovery stored version is invalid".into()))?;
        if version != handoff.expected_execution_version() {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(RecoveryTurnAcceptance::VersionConflict { actual: version });
        }
        let now: i64 = sqlx::query_scalar("SELECT CAST((julianday('now') - 2440587.5) * 86400000.0 AS INTEGER)")
            .fetch_one(&mut *tx).await.map_err(backend_error)?;
        let expiry: Option<i64> = row.try_get("lease_expires_at_ms").map_err(backend_error)?;
        if expiry.is_some_and(|expiry| expiry >= now) {
            tx.rollback().await.map_err(backend_error)?;
            return Ok(RecoveryTurnAcceptance::TurnHeldByAnotherOwner);
        }
        let previous: i64 = row.try_get("fencing_generation").map_err(backend_error)?;
        let generation = previous.checked_add(1).filter(|_| previous >= expected_marker && expected_marker > 0)
            .ok_or_else(|| StorageError::Internal("recovery fence is invalid or exhausted".into()))?;
        let ttl = handoff.lease_ttl().clamp(std::time::Duration::from_secs(1), std::time::Duration::from_hours(24));
        let ttl_ms = i64::try_from(ttl.as_millis()).map_err(|_| StorageError::Internal("recovery lease is invalid".into()))?;
        let expires = now.checked_add(ttl_ms).ok_or_else(|| StorageError::Internal("recovery deadline is invalid".into()))?;
        let fence = FencingToken::from_generation(u64::try_from(generation)
            .map_err(|_| StorageError::Internal("recovery fence is invalid".into()))?);
        let lease = sqlx::query("UPDATE port_executions SET lease_holder = ?, lease_expires_at_ms = ?, fencing_generation = ? WHERE id = ? AND workspace_id = ? AND org_id = ?")
            .bind(handoff.holder()).bind(expires).bind(generation).bind(handoff.execution_id())
            .bind(&handoff.scope().workspace_id).bind(&handoff.scope().org_id)
            .execute(&mut *tx).await.map_err(backend_error)?;
        let marker = sqlx::query("UPDATE port_execution_turn_acceptances SET last_accepted_fencing_generation = ? WHERE execution_id = ? AND workspace_id = ? AND org_id = ? AND last_accepted_fencing_generation = ?")
            .bind(generation).bind(handoff.execution_id()).bind(&handoff.scope().workspace_id).bind(&handoff.scope().org_id)
            .bind(expected_marker).execute(&mut *tx).await.map_err(backend_error)?;
        if lease.rows_affected() != 1 || marker.rows_affected() != 1 {
            return Err(StorageError::Internal("recovery locked rows changed".into()));
        }
        tx.commit().await.map_err(|_| StorageError::AcknowledgementUnknown { operation: "execution_turn_recovery" })?;
        Ok(RecoveryTurnAcceptance::Accepted { fence })
    }.await;
    tracing::Span::current().record(
        "outcome",
        match &result {
            Ok(RecoveryTurnAcceptance::Accepted { .. }) => "accepted",
            Ok(RecoveryTurnAcceptance::CandidateSuperseded) => "candidate_superseded",
            Ok(RecoveryTurnAcceptance::TurnHeldByAnotherOwner) => "turn_held",
            Ok(RecoveryTurnAcceptance::VersionConflict { .. }) => "version_conflict",
            Ok(_) => "unsupported_outcome",
            Err(StorageError::AcknowledgementUnknown { .. }) => "acknowledgement_unknown",
            Err(_) => "error",
        },
    );
    result
}
