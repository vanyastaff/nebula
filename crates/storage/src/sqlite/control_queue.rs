//! SQLite `ControlQueue` + `ExecutionJournalReader` over the port-scoped
//! schema.
//!
//! The queue is a single-consumer status flip (no `FOR UPDATE SKIP
//! LOCKED` equivalent — spec §5 SQLite boundary, documented not hidden).
//! Ids are the raw 16-byte ULID (`BLOB`), never UTF-8-of-ULID. `enqueue`
//! carries the tenant `Scope`; `mark_*` are fenced by the claiming
//! processor.

use std::time::Duration;

use nebula_storage_port::dto::{ControlMsg, JournalEntry};
use nebula_storage_port::store::{ControlQueue, ExecutionJournalReader, ReclaimOutcome};
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
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i64::from(msg.reclaim_count))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlMsg>, StorageError> {
        let mut tx = self.pool.begin().await.map_err(conn_err)?;
        let rows = sqlx::query(
            "SELECT id, execution_id, workspace_id, org_id, command, \
                    w3c_traceparent, reclaim_count \
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
            sqlx::query(
                "UPDATE port_control_queue \
                 SET status = 'Processing', processed_by = ?, \
                     processed_at_ms = ? \
                 WHERE id = ?",
            )
            .bind(processor.as_slice())
            .bind(chrono::Utc::now().timestamp_millis())
            .bind(id_bytes.as_slice())
            .execute(&mut *tx)
            .await
            .map_err(conn_err)?;
            claimed.push(ControlMsg {
                id,
                execution_id: row.try_get("execution_id").map_err(conn_err)?,
                command: decode_command(&row.try_get::<String, _>("command").map_err(conn_err)?)?,
                scope: Scope::new(
                    row.try_get::<String, _>("workspace_id").map_err(conn_err)?,
                    row.try_get::<String, _>("org_id").map_err(conn_err)?,
                ),
                w3c_traceparent: row.try_get("w3c_traceparent").map_err(conn_err)?,
                reclaim_count: row.try_get::<i64, _>("reclaim_count").map_err(conn_err)? as u32,
            });
        }
        tx.commit().await.map_err(conn_err)?;
        Ok(claimed)
    }

    async fn mark_completed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        // Processor-fenced: only the current claimant may transition the
        // row (a reclaimed-then-stale runner is a no-op).
        sqlx::query(
            "UPDATE port_control_queue SET status = 'Completed' \
             WHERE id = ? AND status = 'Processing' AND processed_by = ?",
        )
        .bind(id.as_slice())
        .bind(processor.as_slice())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
        error: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Failed', error_message = ? \
             WHERE id = ? AND status = 'Processing' AND processed_by = ?",
        )
        .bind(error)
        .bind(id.as_slice())
        .bind(processor.as_slice())
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;
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
        let exhausted = sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead' \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND reclaim_count >= ?",
        )
        .bind(cutoff)
        .bind(i64::from(max_reclaim_count))
        .execute(&mut *tx)
        .await
        .map_err(conn_err)?
        .rows_affected();
        // Remaining stale rows → back to Pending, bump reclaim_count.
        let reclaimed = sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Pending', reclaim_count = reclaim_count + 1, \
                 processed_by = NULL, processed_at_ms = NULL \
             WHERE status = 'Processing' AND processed_at_ms < ? \
               AND reclaim_count < ?",
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
