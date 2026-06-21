//! SQLite `ResumeTokenStore` implementation (W-S3c).
//!
//! `consume` uses `DELETE … WHERE token_hash = ? RETURNING *` — a single
//! atomic statement that deletes the row and returns its columns only if it
//! exists (single-use by construction; a second call finds no row and
//! returns `None`).
//!
//! `revoke_on_terminal` deletes all tokens for a `(scope, execution_id)`
//! pair and returns the count removed.  Called by the engine on terminal
//! transitions so orphaned tokens (e.g. execution cancelled while parked)
//! are cleaned up without a TTL sweep.

use nebula_storage_port::Scope;
use nebula_storage_port::StorageError;
use nebula_storage_port::dto::resume_token::{
    ResumeTokenRow, ResumeTokenWaitKind, TokenHash, TokenHashLengthError,
};
use nebula_storage_port::store::ResumeTokenStore;
use sqlx::{Row, SqlitePool};

fn conn_err(e: impl std::fmt::Display) -> StorageError {
    StorageError::Connection(e.to_string())
}

fn deserialize_wait_kind(raw: &str) -> Result<ResumeTokenWaitKind, StorageError> {
    serde_json::from_str(&format!("\"{raw}\""))
        .map_err(|e| StorageError::Serialization(e.to_string()))
}

/// SQLite-backed resume-token store.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`]
/// (which applies the embedded `schema.sql` containing `port_resume_tokens`).
#[derive(Clone, Debug)]
pub struct SqliteResumeTokenStore {
    pool: SqlitePool,
}

impl SqliteResumeTokenStore {
    /// Wrap an existing pool.  The caller installs the port schema.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ResumeTokenStore for SqliteResumeTokenStore {
    async fn consume(
        &self,
        token_hash: &TokenHash,
    ) -> Result<Option<ResumeTokenRow>, StorageError> {
        // Atomic delete-and-return: `DELETE … RETURNING *` in a single
        // statement so there is no window between finding and deleting the
        // row.  SQLite supports RETURNING since 3.35.
        let row = sqlx::query(
            "DELETE FROM port_resume_tokens \
             WHERE token_hash = ? \
             RETURNING token_hash, workspace_id, org_id, execution_id, \
                       node_key, wait_kind, callback_label, created_at, expires_at",
        )
        .bind(token_hash.as_bytes())
        .fetch_optional(&self.pool)
        .await
        .map_err(conn_err)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let raw_hash: Vec<u8> = row.try_get("token_hash").map_err(conn_err)?;
        let hash = TokenHash::try_from_bytes(raw_hash).map_err(|e: TokenHashLengthError| {
            StorageError::Internal(format!("persisted token_hash bad length: {e}"))
        })?;
        let wait_kind_str: String = row.try_get("wait_kind").map_err(conn_err)?;
        let wait_kind = deserialize_wait_kind(&wait_kind_str)?;

        let scope = Scope {
            workspace_id: row.try_get("workspace_id").map_err(conn_err)?,
            org_id: row.try_get("org_id").map_err(conn_err)?,
        };
        Ok(Some(ResumeTokenRow::new(
            hash,
            scope,
            row.try_get("execution_id").map_err(conn_err)?,
            row.try_get("node_key").map_err(conn_err)?,
            wait_kind,
            row.try_get("callback_label").map_err(conn_err)?,
            row.try_get("created_at").map_err(conn_err)?,
            row.try_get("expires_at").map_err(conn_err)?,
        )))
    }

    async fn revoke_on_terminal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<u64, StorageError> {
        let result = sqlx::query(
            "DELETE FROM port_resume_tokens \
             WHERE workspace_id = ? AND org_id = ? AND execution_id = ?",
        )
        .bind(&scope.workspace_id)
        .bind(&scope.org_id)
        .bind(execution_id)
        .execute(&self.pool)
        .await
        .map_err(conn_err)?;

        Ok(result.rows_affected())
    }
}
