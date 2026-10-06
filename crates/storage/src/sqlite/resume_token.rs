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
use nebula_storage_port::dto::{ResumeTokenRow, TokenHash};
use nebula_storage_port::store::ResumeTokenStore;
use sqlx::{Row, SqlitePool};

use crate::sql_error::storage_error;

/// Decode one `port_resume_tokens` row (every column, as selected by
/// `RETURNING` or `SELECT`). The one decoder of the table for this backend.
pub(super) fn decode_resume_token(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ResumeTokenRow, StorageError> {
    let token_hash = TokenHash::try_from_bytes(
        row.try_get::<Vec<u8>, _>("token_hash")
            .map_err(storage_error)?,
    )
    .map_err(|_| StorageError::Corrupt("resume token hash has the wrong length".into()))?;
    let wait_kind = row
        .try_get::<String, _>("wait_kind")
        .map_err(storage_error)?
        .parse()
        .map_err(|_| StorageError::Corrupt("resume token wait kind is unknown".into()))?;
    let scope = Scope {
        workspace_id: row.try_get("workspace_id").map_err(storage_error)?,
        org_id: row.try_get("org_id").map_err(storage_error)?,
    };
    Ok(ResumeTokenRow::new(
        token_hash,
        scope,
        row.try_get("execution_id").map_err(storage_error)?,
        row.try_get("node_key").map_err(storage_error)?,
        wait_kind,
        row.try_get("callback_label").map_err(storage_error)?,
        row.try_get("created_at").map_err(storage_error)?,
        row.try_get("expires_at").map_err(storage_error)?,
    ))
}

/// SQLite-backed resume-token store.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`]
/// (which applies the ordered migration containing `port_resume_tokens`).
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
        .map_err(storage_error)?;

        row.as_ref().map(decode_resume_token).transpose()
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
        .map_err(storage_error)?;

        Ok(result.rows_affected())
    }
}
