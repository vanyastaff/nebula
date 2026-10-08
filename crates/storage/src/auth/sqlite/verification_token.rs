//! SQLite single-use tokens: qualification and consumption are one write.

use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use super::{NOW, instant, micros, optional_instant};
use crate::{
    StorageError,
    auth::{VerificationTokenRepo, VerificationTokenRow},
    sql_error::{storage_error, storage_error_for},
};

/// SQLite verification-token repository on an existing deployment pool.
#[derive(Clone)]
pub struct SqliteVerificationTokenRepo {
    pool: SqlitePool,
}

impl SqliteVerificationTokenRepo {
    /// Bind to the application's admitted pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn consume(
        &self,
        hash: &[u8],
        kind: Option<&str>,
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE verification_tokens SET consumed_at = {NOW}
             WHERE token_hash = ?1 AND (?2 IS NULL OR kind = ?2)
               AND consumed_at IS NULL AND expires_at > {NOW}
               AND EXISTS (SELECT 1 FROM users WHERE id = verification_tokens.user_id AND deleted_at IS NULL)
             RETURNING {COLUMNS}"
        )))
        .bind(hash)
        .bind(kind)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_token)
        .transpose()
    }
}

const COLUMNS: &str = "token_hash, user_id, kind, payload, created_at, expires_at, consumed_at";

fn decode_token(row: SqliteRow) -> Result<VerificationTokenRow, StorageError> {
    let payload: Option<String> = row.try_get("payload").map_err(storage_error)?;
    Ok(VerificationTokenRow {
        token_hash: row.try_get("token_hash").map_err(storage_error)?,
        user_id: row.try_get("user_id").map_err(storage_error)?,
        kind: row.try_get("kind").map_err(storage_error)?,
        payload: payload
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|_| StorageError::Corrupt("invalid verification token payload".into()))
            })
            .transpose()?,
        created_at: instant(&row, "created_at")?,
        expires_at: instant(&row, "expires_at")?,
        consumed_at: optional_instant(&row, "consumed_at")?,
    })
}

#[async_trait::async_trait]
impl VerificationTokenRepo for SqliteVerificationTokenRepo {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn create(&self, token: &VerificationTokenRow) -> Result<(), StorageError> {
        let inserted = sqlx::query(
            "INSERT INTO verification_tokens
             (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at)
             SELECT ?1, id, ?3, ?4, ?5, ?6, ?7 FROM users
             WHERE id = ?2 AND deleted_at IS NULL",
        )
        .bind(&token.token_hash)
        .bind(&token.user_id)
        .bind(&token.kind)
        .bind(token.payload.as_ref().map(serde_json::Value::to_string))
        .bind(token.created_at.timestamp_micros())
        .bind(token.expires_at.timestamp_micros())
        .bind(micros(token.consumed_at))
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("verification_token", error))?
        .rows_affected();
        if inserted == 0 {
            return Err(StorageError::not_found("user", "verification token owner"));
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn consume_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        self.consume(token_hash, None).await
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn consume_by_hash_and_kind(
        &self,
        token_hash: &[u8],
        kind: &str,
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        self.consume(token_hash, Some(kind)).await
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get_by_hash(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<VerificationTokenRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM verification_tokens WHERE token_hash = ?
               AND EXISTS (SELECT 1 FROM users WHERE id = verification_tokens.user_id AND deleted_at IS NULL)"
        )))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_token)
        .transpose()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn cleanup_expired(&self) -> Result<u64, StorageError> {
        Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM verification_tokens WHERE expires_at <= {NOW}
               AND EXISTS (SELECT 1 FROM users WHERE id = verification_tokens.user_id AND deleted_at IS NULL)"
        )))
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn revoke_all_for_user(&self, user_id: &[u8], kind: &str) -> Result<u64, StorageError> {
        Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE verification_tokens SET consumed_at = {NOW}
             WHERE user_id = ? AND kind = ? AND consumed_at IS NULL
               AND EXISTS (SELECT 1 FROM users WHERE id = verification_tokens.user_id AND deleted_at IS NULL)"
        )))
        .bind(user_id)
        .bind(kind)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected())
    }
}
