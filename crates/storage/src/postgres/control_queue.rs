//! Postgres `ControlQueue` + `ExecutionJournalReader` over the
//! port-scoped schema.
//!
//! The claim path uses `FOR UPDATE SKIP LOCKED` so multiple consumers can
//! drain the queue concurrently without double-dispatch (multi-consumer,
//! ADR-0008 §1). Ids are the raw 16-byte ULID (`BYTEA`), never
//! UTF-8-of-ULID. `enqueue` carries the tenant `Scope`; `mark_*` are
//! fenced by the claiming processor.

use std::time::Duration;

use chrono::Utc;
use nebula_storage_port::dto::{ControlMsg, JournalEntry};
use nebula_storage_port::store::{ControlQueue, ExecutionJournalReader, ReclaimOutcome};
use nebula_storage_port::{Scope, StorageError};
use sqlx::{PgPool, Row};

use super::execution::conn_err;

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
impl ControlQueue for PgControlQueue {
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count) \
             VALUES ($1, $2, $3, $4, $5, 'Pending', $6, $7)",
        )
        .bind(msg.id.as_slice())
        .bind(&msg.execution_id)
        .bind(&msg.scope.workspace_id)
        .bind(&msg.scope.org_id)
        .bind(msg.command.as_str())
        .bind(msg.w3c_traceparent.as_deref())
        .bind(i32::try_from(msg.reclaim_count).unwrap_or(i32::MAX))
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
        // FOR UPDATE SKIP LOCKED: concurrent consumers each grab a
        // disjoint set of pending rows without blocking each other.
        // `processed_at_ms` is epoch-millis (`BIGINT`), the same
        // representation the SQLite backend uses — bind the instant from
        // Rust (`now()` SQL would be `TIMESTAMPTZ`) so both dialects
        // stamp and compare the reclaim clock identically.
        let now_ms = Utc::now().timestamp_millis();
        let rows = sqlx::query(
            "UPDATE port_control_queue SET status = 'Processing', \
                    processed_by = $1, processed_at_ms = $2 \
             WHERE id IN ( \
                 SELECT id FROM port_control_queue \
                 WHERE status = 'Pending' \
                 ORDER BY id \
                 LIMIT $3 \
                 FOR UPDATE SKIP LOCKED \
             ) \
             RETURNING id, execution_id, workspace_id, org_id, command, \
                       w3c_traceparent, reclaim_count",
        )
        .bind(processor.as_slice())
        .bind(now_ms)
        .bind(i64::from(batch_size))
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.into_iter()
            .map(|row| {
                let id_bytes: Vec<u8> = row.try_get("id").map_err(conn_err)?;
                Ok(ControlMsg {
                    id: decode_id(&id_bytes)?,
                    execution_id: row.try_get("execution_id").map_err(conn_err)?,
                    command: decode_command(
                        &row.try_get::<String, _>("command").map_err(conn_err)?,
                    )?,
                    scope: Scope::new(
                        row.try_get::<String, _>("workspace_id").map_err(conn_err)?,
                        row.try_get::<String, _>("org_id").map_err(conn_err)?,
                    ),
                    w3c_traceparent: row.try_get("w3c_traceparent").map_err(conn_err)?,
                    reclaim_count: row.try_get::<i32, _>("reclaim_count").map_err(conn_err)? as u32,
                })
            })
            .collect()
    }

    async fn mark_completed(
        &self,
        id: &[u8; 16],
        processor: &[u8; 16],
    ) -> Result<(), StorageError> {
        sqlx::query(
            "UPDATE port_control_queue SET status = 'Completed' \
             WHERE id = $1 AND status = 'Processing' AND processed_by = $2",
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
             SET status = 'Failed', error_message = $1 \
             WHERE id = $2 AND status = 'Processing' AND processed_by = $3",
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
        // `processed_at_ms` is epoch-millis (`BIGINT`) — the same
        // representation and cutoff arithmetic the SQLite backend uses,
        // so reclaim fires at the identical instant on both dialects.
        let cutoff_ms = Utc::now()
            .timestamp_millis()
            .saturating_sub(reclaim_after.as_millis() as i64);
        let exhausted = sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Failed', \
                 error_message = 'reclaim exhausted: presumed dead' \
             WHERE status = 'Processing' AND processed_at_ms < $1 \
               AND reclaim_count >= $2",
        )
        .bind(cutoff_ms)
        .bind(i32::try_from(max_reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        let reclaimed = sqlx::query(
            "UPDATE port_control_queue \
             SET status = 'Pending', reclaim_count = reclaim_count + 1, \
                 processed_by = NULL, processed_at_ms = NULL \
             WHERE status = 'Processing' AND processed_at_ms < $1 \
               AND reclaim_count < $2",
        )
        .bind(cutoff_ms)
        .bind(i32::try_from(max_reclaim_count).unwrap_or(i32::MAX))
        .execute(&self.pool)
        .await
        .map_err(conn_err)?
        .rows_affected();
        Ok(ReclaimOutcome {
            reclaimed,
            exhausted,
        })
    }

    async fn cleanup(&self, _retention: Duration) -> Result<u64, StorageError> {
        // Terminal rows are pruned by the consumer's retention sweep;
        // there is no enqueue timestamp column in the port-scoped schema,
        // so age-based pruning is a deliberate no-op here (parity with the
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
            "SELECT 1 AS ok FROM port_executions \
             WHERE id = $1 AND workspace_id = $2 AND org_id = $3",
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
            "SELECT seq, payload FROM port_execution_journal \
             WHERE execution_id = $1 ORDER BY seq",
        )
        .bind(execution_id)
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.into_iter()
            .map(|r| {
                Ok(JournalEntry {
                    seq: Some(r.try_get::<i64, _>("seq").map_err(conn_err)? as u64),
                    payload: r.try_get("payload").map_err(conn_err)?,
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
             WHERE execution_id = $1 AND seq > $2 ORDER BY seq",
        )
        .bind(execution_id)
        .bind(i64::try_from(after).unwrap_or(i64::MAX))
        .fetch_all(&self.pool)
        .await
        .map_err(conn_err)?;
        rows.into_iter()
            .map(|r| {
                Ok(JournalEntry {
                    seq: Some(r.try_get::<i64, _>("seq").map_err(conn_err)? as u64),
                    payload: r.try_get("payload").map_err(conn_err)?,
                })
            })
            .collect()
    }
}
