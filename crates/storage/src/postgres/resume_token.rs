//! PostgreSQL `ResumeTokenStore` implementation (W-S3c).
//!
//! `consume` uses `DELETE … WHERE token_hash = $1 RETURNING *` — a single
//! atomic statement that deletes the row and returns its columns only if it
//! exists (single-use by construction; a second call finds no row and
//! returns `None`).  The `token_hash` column is `BYTEA PRIMARY KEY`, so
//! the lookup is an exact O(log n) B-tree seek with no collation ambiguity.
//!
//! `revoke_on_terminal` deletes all tokens for a `(scope, execution_id)`
//! pair using the `ix_resume_tokens__org_id_workspace_id_execution_id` index.

use chrono::{DateTime, Utc};
use nebula_storage_port::Scope;
use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{ResumeTokenRow, TokenHash};
use nebula_storage_port::store::ResumeTokenStore;
use sqlx::{PgPool, Row};

use crate::sql_error::storage_error;

/// Decode one `resume_tokens` row (every column, as selected by
/// `RETURNING` or `SELECT`). The one decoder of the table for this backend.
///
/// `created_at`/`expires_at` are `TIMESTAMPTZ`, rendered back to the DTO's
/// RFC 3339 strings.
pub(super) fn decode_resume_token(
    row: &sqlx::postgres::PgRow,
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
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(storage_error)?;
    let expires_at: Option<DateTime<Utc>> = row.try_get("expires_at").map_err(storage_error)?;
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
        created_at.to_rfc3339(),
        expires_at.map(|instant| instant.to_rfc3339()),
    ))
}

/// PostgreSQL-backed resume-token store.
///
/// Wrap a pool whose schema was installed via [`super::init_schema`]
/// (which applies the ordered migration containing `resume_tokens`).
#[derive(Clone, Debug)]
pub struct PgResumeTokenStore {
    pool: PgPool,
}

impl PgResumeTokenStore {
    /// Wrap an existing pool.  The caller installs the port schema.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ResumeTokenStore for PgResumeTokenStore {
    async fn consume(
        &self,
        token_hash: &TokenHash,
    ) -> Result<Option<ResumeTokenRow>, StorageError> {
        // Atomic delete-and-return in a single statement.  The BYTEA
        // primary-key lookup is an exact B-tree seek: no collation path,
        // no encoding ambiguity.
        let row = sqlx::query(
            "DELETE FROM resume_tokens \
             WHERE token_hash = $1 \
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
            "DELETE FROM resume_tokens \
             WHERE org_id = $1 AND workspace_id = $2 AND execution_id = $3",
        )
        .bind(&scope.org_id)
        .bind(&scope.workspace_id)
        .bind(execution_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;

        Ok(result.rows_affected())
    }
}
