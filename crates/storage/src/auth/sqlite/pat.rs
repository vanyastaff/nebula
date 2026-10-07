//! Personal access tokens with atomic, principal-qualified revocation.

use super::{NOW, instant, micros, optional_instant};
use crate::{
    StorageError,
    auth::{PatRepo, PersonalAccessTokenRow},
    sql_error::{storage_error, storage_error_for},
};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

/// SQLite personal access token repository on an existing deployment pool.
#[derive(Clone)]
pub struct SqlitePatRepo {
    pool: SqlitePool,
}

impl SqlitePatRepo {
    /// Bind to the application's admitted pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

const COLUMNS: &str = "id, principal_kind, principal_id, name, prefix, hash, scopes, created_at, last_used_at, expires_at, revoked_at";

fn decode_pat(row: SqliteRow) -> Result<PersonalAccessTokenRow, StorageError> {
    let scopes: String = row.try_get("scopes").map_err(storage_error)?;
    Ok(PersonalAccessTokenRow {
        id: row.try_get("id").map_err(storage_error)?,
        principal_kind: row.try_get("principal_kind").map_err(storage_error)?,
        principal_id: row.try_get("principal_id").map_err(storage_error)?,
        name: row.try_get("name").map_err(storage_error)?,
        prefix: row.try_get("prefix").map_err(storage_error)?,
        hash: row.try_get("hash").map_err(storage_error)?,
        scopes: serde_json::from_str(&scopes)
            .map_err(|_| StorageError::Corrupt("invalid personal access token scopes".into()))?,
        created_at: instant(&row, "created_at")?,
        last_used_at: optional_instant(&row, "last_used_at")?,
        expires_at: optional_instant(&row, "expires_at")?,
        revoked_at: optional_instant(&row, "revoked_at")?,
    })
}

#[async_trait::async_trait]
impl PatRepo for SqlitePatRepo {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn create(&self, pat: &PersonalAccessTokenRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO personal_access_tokens
             (id, principal_kind, principal_id, name, prefix, hash, scopes,
              created_at, last_used_at, expires_at, revoked_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&pat.id)
        .bind(&pat.principal_kind)
        .bind(&pat.principal_id)
        .bind(&pat.name)
        .bind(&pat.prefix)
        .bind(&pat.hash)
        .bind(pat.scopes.to_string())
        .bind(pat.created_at.timestamp_micros())
        .bind(micros(pat.last_used_at))
        .bind(micros(pat.expires_at))
        .bind(micros(pat.revoked_at))
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("pat", error))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get_by_hash(
        &self,
        hash: &[u8],
    ) -> Result<Option<PersonalAccessTokenRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM personal_access_tokens
            WHERE hash = ? AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > {NOW})"
        )))
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_pat)
        .transpose()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn touch(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE personal_access_tokens SET last_used_at = {NOW}
            WHERE id = ? AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > {NOW})"
        )))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn revoke_for_principal(
        &self,
        id: &[u8],
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE personal_access_tokens SET revoked_at = COALESCE(revoked_at, {NOW})
            WHERE id = ? AND principal_kind = ? AND principal_id = ?"
        )))
        .bind(id)
        .bind(principal_kind)
        .bind(principal_id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected()
            == 1)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn list_for_principal(
        &self,
        principal_kind: &str,
        principal_id: &[u8],
    ) -> Result<Vec<PersonalAccessTokenRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM personal_access_tokens
            WHERE principal_kind = ? AND principal_id = ? AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > {NOW}) ORDER BY created_at, id"
        )))
        .bind(principal_kind)
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .into_iter()
        .map(decode_pat)
        .collect()
    }
}
