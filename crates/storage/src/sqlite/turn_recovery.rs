//! Bounded advisory discovery and atomic recovery of durably accepted turns.
//! Instants are INTEGER microseconds since the Unix epoch.
use nebula_storage_port::{
    FencingToken, Scope, StorageError,
    store::{RecoverableTurn, RecoverableTurnPage, RecoveryTurnAcceptance, RecoveryTurnHandoff},
};
use sqlx::{Row, SqlitePool};

use crate::sql_error::storage_error;

/// The database clock in microseconds since the Unix epoch.
const NOW_MICROS: &str = "CAST((julianday('now') - 2440587.5) * 86400000000.0 AS INTEGER)";

#[tracing::instrument(name = "turn_recovery.list", skip_all,
    fields(backend = "sqlite", limit, candidates = tracing::field::Empty))]
pub(super) async fn list(
    pool: &SqlitePool,
    flavor: nebula_core::WorkerFlavorRevisionId,
    after: Option<&str>,
    limit: u32,
) -> Result<RecoverableTurnPage, StorageError> {
    if !(1..=256).contains(&limit) {
        return Err(StorageError::InvalidInput(
            "recovery page limit must be in 1..=256".into(),
        ));
    }
    // Page exact live references first. Live leases still advance the cursor.
    let sql = format!(
        "SELECT a.execution_id, a.org_id, a.workspace_id, a.last_accepted_fencing_generation, \
                e.lease_expires_at, {NOW_MICROS} AS scan_now \
         FROM execution_turn_acceptances a \
         JOIN executions e \
           ON e.org_id = a.org_id AND e.workspace_id = a.workspace_id AND e.id = a.execution_id \
         JOIN execution_revision_references r ON r.execution_id = a.execution_id \
         WHERE r.reference_state = 'live' AND r.worker_flavor_id = ?1 \
           AND (?2 IS NULL OR a.execution_id > ?2) \
         ORDER BY a.execution_id LIMIT ?3"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(flavor.as_bytes().as_slice())
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await
        .map_err(storage_error)?;
    let scanned = rows.len();
    let mut last = None;
    let mut turns = Vec::new();
    for row in rows {
        let execution_id: String = row.try_get("execution_id").map_err(storage_error)?;
        last = Some(execution_id.clone());
        let now: i64 = row.try_get("scan_now").map_err(storage_error)?;
        let expiry: Option<i64> = row.try_get("lease_expires_at").map_err(storage_error)?;
        if expiry.is_none_or(|expiry| expiry < now) {
            let generation: i64 = row
                .try_get("last_accepted_fencing_generation")
                .map_err(storage_error)?;
            let generation = u64::try_from(generation)
                .ok()
                .filter(|generation| *generation > 0)
                .ok_or_else(|| {
                    StorageError::Internal("recovery marker generation is invalid".into())
                })?;
            turns.push(RecoverableTurn::new(
                Scope::new(
                    row.try_get::<String, _>("workspace_id")
                        .map_err(storage_error)?,
                    row.try_get::<String, _>("org_id").map_err(storage_error)?,
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
    let result = accept_in_transaction(pool, handoff).await;
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

async fn accept_in_transaction(
    pool: &SqlitePool,
    handoff: &RecoveryTurnHandoff<'_>,
) -> Result<RecoveryTurnAcceptance, StorageError> {
    let scope = handoff.scope();
    i64::try_from(handoff.expected_execution_version())
        .map_err(|_| StorageError::Internal("recovery execution version is invalid".into()))?;
    let expected_marker = i64::try_from(handoff.expected_accepted_fencing_generation())
        .map_err(|_| StorageError::Internal("recovery marker generation is invalid".into()))?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(storage_error)?;
    let Some(row) = sqlx::query(
        "SELECT version, fencing_generation, lease_expires_at FROM executions \
         WHERE org_id = ? AND workspace_id = ? AND id = ?",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(handoff.execution_id())
    .fetch_optional(&mut *tx)
    .await
    .map_err(storage_error)?
    else {
        tx.rollback().await.map_err(storage_error)?;
        return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
    };
    let marker = sqlx::query(
        "SELECT last_accepted_fencing_generation, source_kind, source_queue_id \
         FROM execution_turn_acceptances a \
         WHERE org_id = ? AND workspace_id = ? AND execution_id = ? \
           AND EXISTS (SELECT 1 FROM execution_revision_references r \
                       WHERE r.execution_id = a.execution_id \
                         AND r.reference_state = 'live' AND r.worker_flavor_id = ?)",
    )
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(handoff.execution_id())
    .bind(handoff.worker_flavor_revision_id().as_bytes().as_slice())
    .fetch_optional(&mut *tx)
    .await
    .map_err(storage_error)?;
    let Some(marker) = marker else {
        tx.rollback().await.map_err(storage_error)?;
        return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
    };
    if marker
        .try_get::<i64, _>("last_accepted_fencing_generation")
        .map_err(storage_error)?
        != expected_marker
    {
        tx.rollback().await.map_err(storage_error)?;
        return Ok(RecoveryTurnAcceptance::CandidateSuperseded);
    }
    let source_kind: String = marker.try_get("source_kind").map_err(storage_error)?;
    let source_row_id: [u8; 16] = marker
        .try_get::<Vec<u8>, _>("source_queue_id")
        .map_err(storage_error)?
        .try_into()
        .map_err(|_| StorageError::Internal("recovery marker source is invalid".into()))?;
    let version = u64::try_from(row.try_get::<i64, _>("version").map_err(storage_error)?)
        .map_err(|_| StorageError::Internal("recovery stored version is invalid".into()))?;
    if version != handoff.expected_execution_version() {
        tx.rollback().await.map_err(storage_error)?;
        return Ok(RecoveryTurnAcceptance::VersionConflict { actual: version });
    }
    let now: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT {NOW_MICROS}")))
        .fetch_one(&mut *tx)
        .await
        .map_err(storage_error)?;
    let expiry: Option<i64> = row.try_get("lease_expires_at").map_err(storage_error)?;
    if expiry.is_some_and(|expiry| expiry >= now) {
        tx.rollback().await.map_err(storage_error)?;
        return Ok(RecoveryTurnAcceptance::TurnHeldByAnotherOwner);
    }
    let previous: i64 = row.try_get("fencing_generation").map_err(storage_error)?;
    let generation = previous
        .checked_add(1)
        .filter(|_| previous >= expected_marker && expected_marker > 0)
        .ok_or_else(|| StorageError::Internal("recovery fence is invalid or exhausted".into()))?;
    let ttl = handoff.lease_ttl().clamp(
        std::time::Duration::from_secs(1),
        std::time::Duration::from_hours(24),
    );
    let expires = i64::try_from(ttl.as_micros())
        .ok()
        .and_then(|ttl| now.checked_add(ttl))
        .ok_or_else(|| StorageError::Internal("recovery deadline is invalid".into()))?;
    let fence = FencingToken::from_generation(
        u64::try_from(generation)
            .map_err(|_| StorageError::Internal("recovery fence is invalid".into()))?,
    );
    let lease = sqlx::query(
        "UPDATE executions SET lease_holder = ?, lease_expires_at = ?, fencing_generation = ? \
         WHERE org_id = ? AND workspace_id = ? AND id = ?",
    )
    .bind(handoff.holder())
    .bind(expires)
    .bind(generation)
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(handoff.execution_id())
    .execute(&mut *tx)
    .await
    .map_err(storage_error)?;
    let marker = sqlx::query(
        "UPDATE execution_turn_acceptances SET last_accepted_fencing_generation = ? \
         WHERE org_id = ? AND workspace_id = ? AND execution_id = ? \
           AND last_accepted_fencing_generation = ?",
    )
    .bind(generation)
    .bind(&scope.org_id)
    .bind(&scope.workspace_id)
    .bind(handoff.execution_id())
    .bind(expected_marker)
    .execute(&mut *tx)
    .await
    .map_err(storage_error)?;
    if lease.rows_affected() != 1 || marker.rows_affected() != 1 {
        return Err(StorageError::Internal(
            "recovery locked rows changed".into(),
        ));
    }
    let payload = crate::control_turn::recovered_turn_payload(
        &source_kind,
        source_row_id,
        handoff.expected_accepted_fencing_generation(),
        fence.generation(),
        chrono::DateTime::from_timestamp_micros(now)
            .ok_or_else(|| StorageError::Internal("recovery timestamp is invalid".into()))?,
    )?;
    super::control_turn::append_observation(&mut tx, scope, handoff.execution_id(), &payload)
        .await?;
    tx.commit()
        .await
        .map_err(|_| StorageError::AcknowledgementUnknown {
            operation: "execution_turn_recovery",
        })?;
    Ok(RecoveryTurnAcceptance::Accepted { fence })
}
