//! Browser-session digest persistence and revocation on SQLite.

use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use super::{NOW, instant, micros, optional_instant};
use crate::{
    StorageError,
    auth::{
        SessionDraft, SessionRepo, SessionRow,
        session_token::{SessionTokenDigest, session_token_digest},
    },
    sql_error::{storage_error, storage_error_for},
};

/// SQLite session repository on an existing deployment pool.
#[derive(Clone)]
pub struct SqliteSessionRepo {
    pool: SqlitePool,
}

impl SqliteSessionRepo {
    /// Bind to the application's admitted pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn decode_session(row: SqliteRow) -> Result<SessionRow, StorageError> {
    let digest: Vec<u8> = row.try_get("token_digest").map_err(storage_error)?;
    Ok(SessionRow {
        token_digest: SessionTokenDigest::from_bytes(
            digest
                .try_into()
                .map_err(|_| StorageError::Corrupt("session digest is not 32 bytes".into()))?,
        ),
        user_id: row.try_get("user_id").map_err(storage_error)?,
        created_at: instant(&row, "created_at")?,
        last_active_at: instant(&row, "last_active_at")?,
        expires_at: instant(&row, "expires_at")?,
        ip_address: row.try_get("ip_address").map_err(storage_error)?,
        user_agent: row.try_get("user_agent").map_err(storage_error)?,
        revoked_at: optional_instant(&row, "revoked_at")?,
    })
}

#[async_trait::async_trait]
impl SessionRepo for SqliteSessionRepo {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn create(
        &self,
        presented_token: &[u8],
        session: &SessionDraft,
    ) -> Result<(), StorageError> {
        let address = session
            .ip_address
            .as_deref()
            .map(|value| {
                value
                    .parse::<std::net::IpAddr>()
                    .map(|address| address.to_string())
                    .map_err(|_| StorageError::InvalidInput("invalid session IP address".into()))
            })
            .transpose()?;
        let inserted = sqlx::query(
            "INSERT INTO sessions
             (token_digest, user_id, created_at, last_active_at, expires_at,
              ip_address, user_agent, revoked_at)
             SELECT ?1, id, ?3, ?4, ?5, ?6, ?7, ?8 FROM users
             WHERE id = ?2 AND deleted_at IS NULL",
        )
        .bind(session_token_digest(presented_token).as_bytes().as_slice())
        .bind(&session.user_id)
        .bind(session.created_at.timestamp_micros())
        .bind(session.last_active_at.timestamp_micros())
        .bind(session.expires_at.timestamp_micros())
        .bind(address)
        .bind(&session.user_agent)
        .bind(micros(session.revoked_at))
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("session", error))?
        .rows_affected();
        if inserted == 0 {
            return Err(StorageError::not_found("user", "session owner"));
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get(&self, presented_token: &[u8]) -> Result<Option<SessionRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT token_digest, user_id, created_at, last_active_at, expires_at,
                    ip_address, user_agent, revoked_at
             FROM sessions
             WHERE token_digest = ? AND revoked_at IS NULL AND expires_at > {NOW}
               AND EXISTS (SELECT 1 FROM users WHERE id = sessions.user_id AND deleted_at IS NULL)"
        )))
        .bind(session_token_digest(presented_token).as_bytes().as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_session)
        .transpose()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn touch(&self, presented_token: &[u8]) -> Result<(), StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE sessions SET last_active_at = {NOW}
            WHERE token_digest = ? AND revoked_at IS NULL AND expires_at > {NOW}
              AND EXISTS (SELECT 1 FROM users WHERE id = sessions.user_id AND deleted_at IS NULL)"
        )))
        .bind(session_token_digest(presented_token).as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn revoke(&self, presented_token: &[u8]) -> Result<(), StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE sessions SET revoked_at = {NOW} WHERE token_digest = ? AND revoked_at IS NULL
               AND EXISTS (SELECT 1 FROM users WHERE id = sessions.user_id AND deleted_at IS NULL)"
        )))
        .bind(session_token_digest(presented_token).as_bytes().as_slice())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn cleanup_expired(&self) -> Result<u64, StorageError> {
        Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM sessions WHERE expires_at <= {NOW}
               AND EXISTS (SELECT 1 FROM users WHERE id = sessions.user_id AND deleted_at IS NULL)"
        )))
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected())
    }
}
