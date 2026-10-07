//! Postgres `ControlQueue` + `ExecutionJournalReader` over
//! `execution_control_queue` and `execution_journal`.
//!
//! The claim path uses `FOR UPDATE SKIP LOCKED` so multiple consumers can
//! drain the queue concurrently without double-dispatch (multi-consumer,
//! multi-consumer claim). Ids are the raw 16-byte ULID (`BYTEA`), never
//! UTF-8-of-ULID. `enqueue` carries the tenant `Scope`; `mark_*` are
//! fenced by the claiming processor. Claim and reclaim instants come from
//! the database clock (`clock_timestamp()`), so every replica stamps and
//! compares them identically.

use std::time::Duration;

use nebula_storage_port::dto::{ControlMsg, JournalEntry};
use nebula_storage_port::store::{
    ClaimGeneration, ControlClaim, ControlClaimToken, ControlQueue, ExecutionJournalReader,
    ReclaimOutcome,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row};

use crate::sql_error::{foreign_key_not_found, storage_error};

/// The longest age a queue sweep measures, in microseconds (100 years):
/// PostgreSQL rejects an `INTERVAL` past its range, and nothing a sweep
/// compares is older.
const MAX_AGE_MICROS: i64 = 100 * 366 * 86_400 * 1_000_000;

/// `age` in microseconds for a `$n * INTERVAL '1 microsecond'` cutoff,
/// clamped to [`MAX_AGE_MICROS`].
pub(super) fn age_micros(age: Duration) -> i64 {
    i64::try_from(age.as_micros()).map_or(MAX_AGE_MICROS, |micros| micros.min(MAX_AGE_MICROS))
}

/// Decode a stored `reclaim_count`; a negative value is corrupt.
pub(super) fn decode_reclaim_count(value: i32) -> Result<u32, StorageError> {
    u32::try_from(value)
        .map_err(|_| StorageError::Corrupt("column `reclaim_count` holds a negative count".into()))
}

/// Encode a message's `reclaim_count` for the `INTEGER` column.
pub(super) fn encode_reclaim_count(value: u32) -> Result<i32, StorageError> {
    i32::try_from(value)
        .map_err(|_| StorageError::InvalidInput("`reclaim_count` exceeds the 32-bit range".into()))
}

/// Encode the `resume_target` of a message as its `JSONB` value.
fn encode_resume_target(msg: &ControlMsg) -> Result<Option<serde_json::Value>, StorageError> {
    msg.resume_target
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(StorageError::from)
}

/// Decode a nullable `resume_target` `JSONB` column.
pub(super) fn decode_resume_target<T: serde::de::DeserializeOwned>(
    row: &sqlx::postgres::PgRow,
) -> Result<Option<T>, StorageError> {
    row.try_get::<Option<serde_json::Value>, _>("resume_target")
        .map_err(storage_error)?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| StorageError::Corrupt("column `resume_target` is not a resume target".into()))
}

/// Insert one `Pending` control message — the single insert shape of
/// `enqueue`, the execution outbox and the resume producer. A message that
/// names no execution in its tenant is `NotFound { entity: "execution" }`;
/// a taken id is `Duplicate { entity: "control_queue" }`.
pub(super) async fn insert_control_message<'e, E>(
    executor: E,
    msg: &ControlMsg,
) -> Result<(), StorageError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO execution_control_queue \
         (org_id, workspace_id, execution_id, id, command, status, \
          resume_target, w3c_traceparent, reclaim_count) \
         VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7, $8)",
    )
    .bind(&msg.scope.org_id)
    .bind(&msg.scope.workspace_id)
    .bind(&msg.execution_id)
    .bind(msg.id.as_slice())
    .bind(msg.command.as_str())
    .bind(encode_resume_target(msg)?)
    .bind(msg.w3c_traceparent.as_deref())
    .bind(encode_reclaim_count(msg.reclaim_count)?)
    .execute(executor)
    .await
    .map_err(
        |error| match foreign_key_not_found(error, "execution", &msg.execution_id) {
            StorageError::Duplicate { detail, .. } => StorageError::Duplicate {
                entity: "control_queue",
                detail,
            },
            other => other,
        },
    )?;
    Ok(())
}

fn decode_claim(row: sqlx::postgres::PgRow) -> Result<ControlClaim, StorageError> {
    let id = decode_id(&row.try_get::<Vec<u8>, _>("id").map_err(storage_error)?)?;
    let generation = row.try_get("claim_generation").map_err(storage_error)?;
    let resume_target = decode_resume_target(&row)?;
    let msg = ControlMsg {
        id,
        execution_id: row.try_get("execution_id").map_err(storage_error)?,
        scope: Scope::new(
            row.try_get::<String, _>("workspace_id")
                .map_err(storage_error)?,
            row.try_get::<String, _>("org_id").map_err(storage_error)?,
        ),
        command: decode_command(&row.try_get::<String, _>("command").map_err(storage_error)?)?,
        w3c_traceparent: row.try_get("w3c_traceparent").map_err(storage_error)?,
        reclaim_count: decode_reclaim_count(row.try_get("reclaim_count").map_err(storage_error)?)?,
        resume_target,
    };
    let token = ControlClaimToken::new(id, decode_generation(generation, &id)?, msg.scope.clone());
    Ok(ControlClaim { msg, token })
}

/// Postgres-backed durable-outbox handle.
#[derive(Clone, Debug)]
pub struct PgControlQueue {
    pool: PgPool,
}

impl PgControlQueue {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Explain why a fenced acknowledgement matched no row.
    ///
    /// The fenced `UPDATE` reports only "zero rows", which conflates "the row
    /// is gone" with "this token is stale". An absent row is a lost write; a
    /// superseded token is the fence doing its job. The follow-up read runs on
    /// the failure path only.
    async fn unacknowledgeable(
        &self,
        claim: &ControlClaimToken,
    ) -> Result<StorageError, StorageError> {
        let exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM execution_control_queue \
                 WHERE id = $1 AND org_id = $2 AND workspace_id = $3",
        )
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().org_id)
        .bind(&claim.scope().workspace_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(if exists.is_some() {
            StorageError::FencedOut {
                entity: "control_queue",
                id: hex_id(claim.row_id()),
            }
        } else {
            StorageError::NotFound {
                entity: "control_queue",
                id: hex_id(claim.row_id()),
            }
        })
    }
}

/// Hex-encode a 16-byte ULID for `StorageError` ids.
fn hex_id(id: &[u8; 16]) -> String {
    id.iter().fold(String::with_capacity(32), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Widen a persisted generation to the port's `u64`.
///
/// A negative value is persisted corruption, and treating it as `0` would make
/// a stale token match. Fail closed instead.
fn decode_generation(generation: i64, id: &[u8; 16]) -> Result<ClaimGeneration, StorageError> {
    u64::try_from(generation)
        .map(ClaimGeneration::new)
        .map_err(|_| {
            StorageError::Serialization(format!(
                "invalid control_queue claim_generation {generation} (id={})",
                hex_id(id)
            ))
        })
}

/// Narrow a token's generation for binding against the `BIGINT` column.
///
/// A generation beyond `i64::MAX` cannot name a persisted row, so binding a
/// saturated value would fence against the wrong row. Fail closed instead.
fn generation_bind(claim: &ControlClaimToken) -> Result<i64, StorageError> {
    i64::try_from(claim.generation().get()).map_err(|_| {
        StorageError::Serialization(format!(
            "control_queue claim generation {} exceeds the persisted range (id={})",
            claim.generation(),
            hex_id(claim.row_id())
        ))
    })
}

fn decode_command(s: &str) -> Result<nebula_storage_port::dto::ControlCommand, StorageError> {
    use nebula_storage_port::dto::ControlCommand as C;
    match s {
        "Start" => Ok(C::Start),
        "Cancel" => Ok(C::Cancel),
        "Terminate" => Ok(C::Terminate),
        "Resume" => Ok(C::Resume),
        "Restart" => Ok(C::Restart),
        _ => Err(StorageError::Serialization(
            "column `command` holds an unknown control command".into(),
        )),
    }
}

fn decode_id(bytes: &[u8]) -> Result<[u8; 16], StorageError> {
    <[u8; 16]>::try_from(bytes).map_err(|_| {
        StorageError::Serialization(format!(
            "control-queue id must be 16 bytes, got {}",
            bytes.len()
        ))
    })
}

#[async_trait::async_trait]
impl ControlQueue for PgControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        insert_control_message(&self.pool, msg).await
    }

    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        // FOR UPDATE SKIP LOCKED: concurrent consumers each grab a
        // disjoint set of pending rows without blocking each other.
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let rows = sqlx::query(
            "UPDATE execution_control_queue SET status = 'Processing', \
                    processed_by = $1, processed_at = clock_timestamp(), \
                    claim_generation = claim_generation + 1 \
             WHERE id IN ( \
                 SELECT id FROM execution_control_queue \
                 WHERE status = 'Pending' \
                 ORDER BY id \
                 LIMIT $2 \
                 FOR UPDATE SKIP LOCKED \
             ) \
             RETURNING id, execution_id, workspace_id, org_id, command, \
                       w3c_traceparent, reclaim_count, resume_target, \
                       claim_generation",
        )
        .bind(processor.as_slice())
        .bind(i64::from(batch_size.clamp(1, 256)))
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let claims = rows
            .into_iter()
            .map(decode_claim)
            .collect::<Result<Vec<_>, StorageError>>()?;
        tx.commit().await.map_err(storage_error)?;
        Ok(claims)
    }

    async fn claim_pending_for_flavor(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
        worker_flavor: nebula_core::WorkerFlavorRevisionId,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let rows = sqlx::query(
            "UPDATE execution_control_queue SET status = 'Processing', \
                 processed_by = $1, processed_at = clock_timestamp(), \
                 claim_generation = claim_generation + 1 \
             WHERE id IN ( \
                 SELECT c.id FROM execution_control_queue c \
                 WHERE c.status = 'Pending' AND EXISTS ( \
                     SELECT 1 FROM execution_revision_references r \
                     WHERE r.org_id = c.org_id AND r.workspace_id = c.workspace_id \
                       AND r.execution_id = c.execution_id \
                       AND r.worker_flavor_id = $3 AND r.reference_state = 'live' \
                 ) ORDER BY c.id LIMIT $2 FOR UPDATE OF c SKIP LOCKED \
             ) RETURNING id, execution_id, workspace_id, org_id, command, \
                 w3c_traceparent, reclaim_count, resume_target, claim_generation",
        )
        .bind(processor.as_slice())
        .bind(i64::from(batch_size.clamp(1, 256)))
        .bind(worker_flavor.as_bytes().as_slice())
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        let claims = rows
            .into_iter()
            .map(decode_claim)
            .collect::<Result<Vec<_>, StorageError>>()?;
        tx.commit().await.map_err(storage_error)?;
        tracing::debug!(
            claimed = claims.len(),
            "claimed exact-flavor control commands"
        );
        Ok(claims)
    }

    async fn mark_completed(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        let rows_updated = sqlx::query(
            "UPDATE execution_control_queue SET status = 'Completed' \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 \
               AND status = 'Processing' AND claim_generation = $4",
        )
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().workspace_id)
        .bind(&claim.scope().org_id)
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if rows_updated == 0 {
            return Err(self.unacknowledgeable(claim).await?);
        }
        Ok(())
    }

    async fn mark_failed(
        &self,
        claim: &ControlClaimToken,
        error: &str,
    ) -> Result<(), StorageError> {
        let rows_updated = sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Failed', error_message = $1 \
             WHERE id = $2 AND workspace_id = $3 AND org_id = $4 \
               AND status = 'Processing' AND claim_generation = $5",
        )
        .bind(error)
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().workspace_id)
        .bind(&claim.scope().org_id)
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if rows_updated == 0 {
            return Err(self.unacknowledgeable(claim).await?);
        }
        Ok(())
    }

    async fn release_claim(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        // Clear `processed_by` / `processed_at` along with the status: a row
        // returned to `Pending` must look unclaimed, or an immediate re-claim
        // would carry stale bookkeeping into the reclaim sweep's staleness
        // check. `reclaim_count` is deliberately untouched — this is a retry,
        // not a symptom of a stuck row.
        let rows_updated = sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Pending', processed_by = NULL, processed_at = NULL \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3 \
               AND status = 'Processing' AND claim_generation = $4",
        )
        .bind(claim.row_id().as_slice())
        .bind(&claim.scope().workspace_id)
        .bind(&claim.scope().org_id)
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if rows_updated == 0 {
            return Err(self.unacknowledgeable(claim).await?);
        }
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError> {
        // A claim is stuck once the database clock passed its claim instant
        // by `reclaim_after`; the same clock stamped `processed_at`.
        let reclaim_after = age_micros(reclaim_after);
        // A `command = 'Resume'` row is EXEMPT from the exhaust budget
        // (ADR-0099 W-S3b): a Resume does no work of its own and cannot
        // poison-loop, so the budget must never force-Fail it. Engine liveness
        // (`acquire_lease`) and the wait's own timeout are the only terminal
        // authorities for a parked Resume. The REDELIVER branch widens to keep
        // an exempt Resume redelivering (observably) past `reclaim_count >= max`
        // rather than wedging in `Processing`.
        let exhausted = sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead' \
             WHERE status = 'Processing' \
               AND processed_at < clock_timestamp() - $1 * INTERVAL '1 microsecond' \
               AND reclaim_count >= $2 AND command <> 'Resume'",
        )
        .bind(reclaim_after)
        .bind(i32::try_from(max_reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected();
        // `OR command = 'Resume'` is the budget-exemption complement of the
        // exhaust branch (ADR-0099 W-S3b): an exempt Resume at
        // `reclaim_count >= max` would otherwise match neither branch and stay
        // stuck `Processing` forever; this keeps it redeliverable.
        let reclaimed = sqlx::query(
            "UPDATE execution_control_queue \
             SET status = 'Pending', reclaim_count = reclaim_count + 1, \
                 processed_by = NULL, processed_at = NULL \
             WHERE status = 'Processing' \
               AND processed_at < clock_timestamp() - $1 * INTERVAL '1 microsecond' \
               AND (reclaim_count < $2 OR command = 'Resume')",
        )
        .bind(reclaim_after)
        .bind(i32::try_from(max_reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected();
        Ok(ReclaimOutcome {
            reclaimed,
            exhausted,
        })
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // Terminal rows are pruned by the consumer's retention sweep;
        // `execution_control_queue` has no enqueue instant and its rows are
        // purged with their execution, so age-based pruning is a deliberate no-op here (parity with the
        // sqlite and in-memory backends — not an unimplemented stub).
        Ok(0)
    }
}

/// Postgres-backed journal reader.
#[derive(Clone, Debug)]
pub struct PgJournalReader {
    pool: PgPool,
}

impl PgJournalReader {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Scope-guard: confirm the execution is visible in `scope` before
    /// returning its journal (a cross-tenant read yields an empty
    /// journal, never another tenant's entries).
    async fn scope_ok(&self, scope: &Scope, execution_id: &str) -> Result<bool, StorageError> {
        let row = sqlx::query(
            "SELECT 1 AS ok FROM executions \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3",
        )
        .bind(execution_id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(row.is_some())
    }
}

#[async_trait::async_trait]
impl ExecutionJournalReader for PgJournalReader {
    async fn get_journal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        if !self.scope_ok(scope, execution_id).await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT seq, payload FROM execution_journal \
             WHERE execution_id = $1 ORDER BY seq",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        rows.into_iter()
            .map(|r| {
                Ok(JournalEntry {
                    seq: Some(r.try_get::<i64, _>("seq").map_err(storage_error)? as u64),
                    payload: r.try_get("payload").map_err(storage_error)?,
                })
            })
            .collect()
    }

    async fn list_after(
        &self,
        scope: &Scope,
        execution_id: &str,
        after: u64,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        if !self.scope_ok(scope, execution_id).await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT seq, payload FROM execution_journal \
             WHERE execution_id = $1 AND seq > $2 ORDER BY seq",
        )
        .bind(execution_id)
        .bind(i64::try_from(after).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        rows.into_iter()
            .map(|r| {
                Ok(JournalEntry {
                    seq: Some(r.try_get::<i64, _>("seq").map_err(storage_error)? as u64),
                    payload: r.try_get("payload").map_err(storage_error)?,
                })
            })
            .collect()
    }
}
