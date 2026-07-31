//! SQLite `ControlQueue` + `ExecutionJournalReader` over the port-scoped
//! schema.
//!
//! The queue is a single-consumer status flip (no `FOR UPDATE SKIP
//! LOCKED` equivalent — spec §5 SQLite boundary, documented not hidden).
//! Ids are the raw 16-byte ULID (`BLOB`), never UTF-8-of-ULID. `enqueue`
//! carries the tenant `Scope`; `mark_*` are fenced by the claiming
//! processor.

use std::time::Duration;

use nebula_storage_port::dto::{ControlMsg, JournalEntry, ResumeTarget};
use nebula_storage_port::store::{
    ClaimGeneration, ControlClaim, ControlClaimToken, ControlQueue, ExecutionJournalReader,
    ReclaimOutcome,
};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{Row, SqlitePool};

use super::execution::conn_err;

/// SQLite-backed durable-outbox handle.
#[derive(Clone, Debug)]
pub struct SqliteControlQueue {
    pool: SqlitePool,
}

impl SqliteControlQueue {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
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
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM port_control_queue WHERE id = ?")
                .bind(claim.row_id().as_slice())
                .fetch_optional(&self.pool)
                .await
                .map_err(conn_err)?;
        Ok(if exists.is_some() {
            StorageError::FencedOut {
                entity: "control_queue",
                id: ulid_hex(claim.row_id()),
            }
        } else {
            StorageError::NotFound {
                entity: "control_queue",
                id: ulid_hex(claim.row_id()),
            }
        })
    }
}

/// Hex-encode a 16-byte ULID without the optional `hex` crate.
fn ulid_hex(id: &[u8; 16]) -> String {
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
                ulid_hex(id)
            ))
        })
}

/// Narrow a token's generation for binding against the `INTEGER` column.
///
/// A generation beyond `i64::MAX` cannot name a persisted row, so binding a
/// saturated value would fence against the wrong row. Fail closed instead.
fn generation_bind(claim: &ControlClaimToken) -> Result<i64, StorageError> {
    i64::try_from(claim.generation().get()).map_err(|_| {
        StorageError::Serialization(format!(
            "control_queue claim generation {} exceeds the persisted range (id={})",
            claim.generation(),
            ulid_hex(claim.row_id())
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
        other => Err(StorageError::Serialization(format!(
            "unknown control command: {other}"
        ))),
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
impl ControlQueue for SqliteControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        let resume_target_json = msg
            .resume_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count, resume_target) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?, ?)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i64::from(msg.reclaim_count))
        .bind(resume_target_json)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlClaim>, StorageError> {
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let rows = sqlx::query(
            "SELECT id, execution_id, workspace_id, org_id, command, \
                    w3c_traceparent, reclaim_count, resume_target \
             FROM port_control_queue WHERE status = 'Pending' \
             ORDER BY id LIMIT ?",
        )
        .bind(i64::from(batch_size))
        .fetch_all(&mut *tx)
        .await
        .map_err(conn_err)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
            let id = decode_id(&id_bytes)?;
            // The claim flip is conditional on the row still being
            // `Pending`: a concurrent consumer or a reclaim sweep can
            // move it to `Processing`/`Pending`-with-bumped-reclaim
            // between the SELECT above and this UPDATE. Without the
            // `AND status = 'Pending'` guard the UPDATE would be a no-op
            // yet the message would still be pushed — returning work this
            // worker does not actually own (a double-claim). Only push
            // when this UPDATE actually won the row (`rows_affected == 1`).
            // `claim_generation` is incremented by the same statement that
            // makes the row `Processing`, and `RETURNING` hands back the value
            // this claim minted — no separate read a concurrent claim could
            // interleave with.
            let minted: Option<i64> = sqlx::query_scalar(
                "UPDATE port_control_queue \
                 SET status = 'Processing', processed_by = ?, \
                     processed_at_ms = ?, \
                     claim_generation = claim_generation + 1 \
                 WHERE id = ? AND status = 'Pending' \
                 RETURNING claim_generation",
            )
            .bind(processor.as_slice())
            .bind(chrono::Utc::now().timestamp_millis())
            .bind(id_bytes.as_slice())
            .fetch_optional(&mut *tx)
            .await
            .map_err(conn_err)?;
            let Some(generation) = minted else {
                continue;
            };
            let resume_target: Option<ResumeTarget> = row
                .try_get::<Option<String>, _>("resume_target")
                .map_err(conn_err)?
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let msg = ControlMsg {
                id,
                execution_id: row.try_get("execution_id").map_err(conn_err)?,
                command: decode_command(&row.try_get::<String, _>("command").map_err(conn_err)?)?,
                scope: Scope::new(
                    row.try_get::<String, _>("workspace_id").map_err(conn_err)?,
                    row.try_get::<String, _>("org_id").map_err(conn_err)?,
                ),
                w3c_traceparent: row.try_get("w3c_traceparent").map_err(conn_err)?,
                reclaim_count: row.try_get::<i64, _>("reclaim_count").map_err(conn_err)? as u32,
                resume_target,
            };
            claimed.push(ControlClaim {
                msg,
                token: ControlClaimToken::new(id, decode_generation(generation, &id)?),
            });
        }
        tx.commit().await.map_err(conn_err)?;
        Ok(claimed)
    }

    async fn mark_completed(&self, claim: &ControlClaimToken) -> Result<(), StorageError> {
        let rows_updated = sqlx::query(
            "UPDATE port_control_queue SET status = 'Completed' \
             WHERE id = ? AND status = 'Processing' AND claim_generation = ?",
        )
        .bind(claim.row_id().as_slice())
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
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
            "UPDATE port_control_queue \
             SET status = 'Failed', error_message = ? \
             WHERE id = ? AND status = 'Processing' AND claim_generation = ?",
        )
        .bind(error)
        .bind(claim.row_id().as_slice())
        .bind(generation_bind(claim)?)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
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
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::try_from(reclaim_after.as_millis()).unwrap_or(i64::MAX);
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        // Exhausted rows (past the reclaim budget) → Failed.
        //
        // A `command = 'Resume'` row is EXEMPT (ADR-0099 W-S3b): a Resume does
        // no work of its own and cannot poison-loop, so the reclaim budget must
        // never force-Fail it. Engine liveness (`acquire_lease`'s dead-vs-live
        // oracle) and the wait's own timeout are the only terminal authorities
        // for a parked Resume. The paired REDELIVER branch widens to catch the
        // exempt Resume at `reclaim_count >= max`, so it keeps redelivering
        // (observably) rather than wedging in `Processing`.
        let exhausted = sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead' \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND reclaim_count >= ? AND command <> 'Resume'",
        )
        .bind(cutoff)
        .bind(i64::from(max_reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected();
        // Remaining stale rows → back to Pending, bump reclaim_count. A
        // `command = 'Resume'` row redelivers regardless of `reclaim_count`
        // (the `OR command = 'Resume'` clause), the budget-exemption complement
        // of the exhaust branch above (ADR-0099 W-S3b) — without it an exempt
        // Resume at `reclaim_count >= max` would match neither branch and stay
        // stuck `Processing` forever.
        let reclaimed = sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Pending', reclaim_count = reclaim_count + 1, \
                 processed_by = NULL, processed_at_ms = NULL \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND (reclaim_count < ? OR command = 'Resume')",
        )
        .bind(cutoff)
        .bind(i64::from(max_reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected();
        tx.commit().await.map_err(conn_err)?;
        Ok(ReclaimOutcome {
            reclaimed,
            exhausted,
        })
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // Terminal rows are pruned by the consumer's retention sweep;
        // there is no enqueue timestamp column in the port-scoped schema,
        // so age-based pruning is a no-op here.
        Ok(0)
    }
}

/// SQLite-backed journal reader.
#[derive(Clone, Debug)]
pub struct SqliteJournalReader {
    pool: SqlitePool,
}

impl SqliteJournalReader {
    /// Wrap a pool whose schema was installed via [`super::init_schema`].
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Scope-guard: the port-scoped journal table has no scope columns
    /// (journal rows are children of an execution), so a read first
    /// confirms the execution is visible in `scope`. A cross-tenant read
    /// then yields an empty journal, never another tenant's entries.
    async fn scope_ok(&self, scope: &Scope, execution_id: &str) -> Result<bool, StorageError> {
        let row = sqlx::query(
            "SELECT 1 AS ok FROM port_executions \
             WHERE id = ? AND workspace_id = ? AND org_id = ?",
        )
        .bind(execution_id)
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(row.is_some())
    }
}

#[async_trait::async_trait]
impl ExecutionJournalReader for SqliteJournalReader {
    async fn get_journal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Vec<JournalEntry>, StorageError> {
        if !self.scope_ok(scope, execution_id).await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT seq, payload FROM port_execution_journal \
             WHERE execution_id = ? ORDER BY seq",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.into_iter()
            .map(|r| {
                let payload: String = r.try_get("payload").map_err(conn_err)?;
                Ok(JournalEntry {
                    seq: Some(r.try_get::<i64, _>("seq").map_err(conn_err)? as u64),
                    payload: serde_json::from_str(&payload)?,
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
            "SELECT seq, payload FROM port_execution_journal \
             WHERE execution_id = ? AND seq > ? ORDER BY seq",
        )
        .bind(execution_id)
        .bind(i64::try_from(after).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.into_iter()
            .map(|r| {
                let payload: String = r.try_get("payload").map_err(conn_err)?;
                Ok(JournalEntry {
                    seq: Some(r.try_get::<i64, _>("seq").map_err(conn_err)? as u64),
                    payload: serde_json::from_str(&payload)?,
                })
            })
            .collect()
    }
}
