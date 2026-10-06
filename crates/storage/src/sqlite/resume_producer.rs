//! SQLite [`ResumeProducer`] implementation (ADR-0099 W-S3d, Option B1).
//!
//! `peek` is a read-only `SELECT … WHERE token_hash = ?` — no delete.
//!
//! `consume_and_enqueue_resume` runs the token DELETE and the control-queue
//! INSERT in ONE transaction: `BEGIN IMMEDIATE` takes the write lock up front
//! (same single-writer contract as [`super::execution::SqliteExecutionStore`]),
//! `DELETE … RETURNING token_hash` is the single-use replay gate (zero rows
//! deleted → `Ok(false)`, rollback), and on exactly one deleted row the
//! `Resume` control message is inserted with bindings byte-identical to the
//! execution-store outbox append. A transient fault rolls the tx back, leaving
//! the token live for the caller's retry — the durability gap the prior
//! consume-then-enqueue handler had.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{ControlCommand, ControlMsg};
use nebula_storage_port::dto::{ResumeTokenRow, TokenHash};
use nebula_storage_port::store::ResumeProducer;
use sqlx::SqlitePool;

use super::resume_token::decode_resume_token;
use crate::sql_error::storage_error;

/// SQLite-backed resume producer.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`] (the same
/// pool the execution store and control queue use, so the DELETE and INSERT
/// share one connection/transaction).
#[derive(Clone, Debug)]
pub struct SqliteResumeProducer {
    pool: SqlitePool,
}

impl SqliteResumeProducer {
    /// Wrap an existing pool. The caller installs the port schema.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ResumeProducer for SqliteResumeProducer {
    async fn peek(&self, token_hash: &TokenHash) -> Result<Option<ResumeTokenRow>, StorageError> {
        let row = sqlx::query(
            "SELECT token_hash, workspace_id, org_id, execution_id, \
                    node_key, wait_kind, callback_label, created_at, expires_at \
             FROM port_resume_tokens \
             WHERE token_hash = ?",
        )
        .bind(token_hash.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;

        match row {
            None => Ok(None),
            Some(row) => Ok(Some(decode_resume_token(&row)?)),
        }
    }

    async fn consume_and_enqueue_resume(
        &self,
        token_hash: &TokenHash,
        resume_msg: &ControlMsg,
    ) -> Result<bool, StorageError> {
        // Fail-closed at the boundary: this producer enqueues ONLY `Resume`.
        // Checked BEFORE any mutation and release-enforced (unlike a
        // `debug_assert!`), so a misused command can never burn a token.
        if resume_msg.command != ControlCommand::Resume {
            return Err(StorageError::Internal(
                "ResumeProducer requires a Resume command".to_owned(),
            ));
        }

        // BEGIN IMMEDIATE takes the write lock up front so the DELETE + INSERT
        // pair is atomic against the single writer (spec §5 SQLite contract;
        // mirrors `SqliteExecutionStore::commit`).
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        // sqlx already opened a tx, so the lock-upgrade hint may fail with
        // "cannot start a transaction within a transaction" — best-effort only.
        let _hint = sqlx::query("BEGIN IMMEDIATE").execute(&mut *tx).await;

        // `DELETE … RETURNING` (rows-affected == 1) IS the single-use replay
        // gate: a raced/replayed/absent hash deletes zero rows.
        let deleted = sqlx::query(
            "DELETE FROM port_resume_tokens \
             WHERE token_hash = ? \
             RETURNING token_hash",
        )
        .bind(token_hash.as_bytes())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;

        if deleted.is_none() {
            // Zero rows deleted — nothing to enqueue. Roll back the empty tx.
            tx.rollback().await.map_err(storage_error)?;
            return Ok(false);
        }

        // Exactly one row deleted — enqueue the Resume in the SAME transaction.
        // Bindings byte-identical to the execution-store outbox append.
        let resume_target_json: Option<String> = resume_msg
            .resume_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(StorageError::from)?;
        sqlx::query(
            "INSERT INTO port_control_queue \
             (id, execution_id, workspace_id, org_id, command, status, \
              w3c_traceparent, reclaim_count, resume_target) \
             VALUES (?, ?, ?, ?, ?, 'Pending', ?, ?, ?)",
        )
        .bind(resume_msg.id.as_slice())
        .bind(&resume_msg.execution_id)
        .bind(&resume_msg.scope.workspace_id)
        .bind(&resume_msg.scope.org_id)
        .bind(resume_msg.command.as_str())
        .bind(resume_msg.w3c_traceparent.as_deref())
        .bind(i64::from(resume_msg.reclaim_count))
        .bind(resume_target_json)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;

        tx.commit().await.map_err(storage_error)?;
        Ok(true)
    }
}
