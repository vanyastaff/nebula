//! Bounded OAuth-state admission on SQLite's shared database writer.
//!
//! Acquire the writer before counting or inserting, without SQLite busy waits.
//! The deployment pool restores connection policy even if admission is cancelled.

use super::{NOW, instant, optional_instant};
use crate::sqlite::DeploymentPool;
use crate::{
    StorageError,
    auth::{OAUTH_STATE_CAPACITY, OAuthStateAdmission, OAuthStateRepo, OAuthStateRow},
    sql_error::{storage_error, storage_error_for},
};
use sqlx::{Connection, Row, SqlitePool, sqlite::SqliteRow};

const COLUMNS: &str =
    "state, provider, code_verifier, redirect_uri, created_at, expires_at, consumed_at";

/// SQLite provider-qualified state admission and single-use consumption.
#[derive(Clone)]
pub struct SqliteOAuthStateRepo {
    pool: SqlitePool,
}

impl SqliteOAuthStateRepo {
    /// Bind to a pool with cancellation-safe release policy restoration.
    #[must_use]
    pub fn new(deployment: &DeploymentPool) -> Self {
        Self {
            pool: deployment.pool().clone(),
        }
    }

    async fn consume(
        &self,
        state: &str,
        provider: Option<&str>,
    ) -> Result<Option<OAuthStateRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE oauth_states SET consumed_at = {NOW}
             WHERE state = ?1 AND (?2 IS NULL OR provider = ?2)
               AND consumed_at IS NULL AND expires_at > {NOW}
             RETURNING {COLUMNS}"
        )))
        .bind(state)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_state)
        .transpose()
    }
}

fn decode_state(row: SqliteRow) -> Result<OAuthStateRow, StorageError> {
    Ok(OAuthStateRow {
        state: row.try_get("state").map_err(storage_error)?,
        provider: row.try_get("provider").map_err(storage_error)?,
        code_verifier: row.try_get("code_verifier").map_err(storage_error)?,
        redirect_uri: row.try_get("redirect_uri").map_err(storage_error)?,
        created_at: instant(&row, "created_at")?,
        expires_at: instant(&row, "expires_at")?,
        consumed_at: optional_instant(&row, "consumed_at")?,
    })
}

fn is_busy(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.try_downcast_ref::<sqlx::sqlite::SqliteError>())
        .and_then(sqlx::error::DatabaseError::code)
        .and_then(|code| code.parse::<u32>().ok())
        .is_some_and(|code| matches!(code & 0xff, 5 | 6))
}

#[async_trait::async_trait]
impl OAuthStateRepo for SqliteOAuthStateRepo {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn admit(&self, state: &OAuthStateRow) -> Result<OAuthStateAdmission, StorageError> {
        if state.state.is_empty()
            || state.provider.is_empty()
            || state.code_verifier.is_empty()
            || state.consumed_at.is_some()
        {
            return Err(StorageError::Internal(
                "invalid OAuth state admission".into(),
            ));
        }
        // Waiting for a free pool slot holds no database connection. Allow a
        // release callback to finish without converting every adjacent request
        // into contention; never wait inside SQLite's physical busy handler.
        let mut connection =
            match tokio::time::timeout(std::time::Duration::from_millis(25), self.pool.acquire())
                .await
            {
                Ok(Ok(connection)) => connection,
                Ok(Err(error)) => return Err(storage_error(error)),
                Err(_) => {
                    tracing::debug!(outcome = "contended", "OAuth state admission refused");
                    return Ok(OAuthStateAdmission::Contended);
                },
            };
        sqlx::query("PRAGMA busy_timeout = 0")
            .execute(&mut *connection)
            .await
            .map_err(storage_error)?;
        let mut tx = match connection.begin_with("BEGIN IMMEDIATE").await {
            Ok(tx) => tx,
            Err(error) if !is_busy(&error) => return Err(storage_error(error)),
            Err(_) => {
                tracing::debug!(outcome = "contended", "OAuth state admission refused");
                return Ok(OAuthStateAdmission::Contended);
            },
        };
        let result: Result<u64, sqlx::Error> = async {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM oauth_states WHERE expires_at <= {NOW}"
            )))
            .execute(&mut *tx)
            .await?;
            let inserted = sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO oauth_states
            (state, provider, code_verifier, redirect_uri, created_at, expires_at)
            SELECT ?1, ?2, ?3, ?4, ?5, ?6
            WHERE (SELECT count(*) FROM (SELECT 1 FROM oauth_states
                WHERE consumed_at IS NULL AND expires_at > {NOW} LIMIT ?7)) < ?7"
            )))
            .bind(&state.state)
            .bind(&state.provider)
            .bind(&state.code_verifier)
            .bind(&state.redirect_uri)
            .bind(state.created_at.timestamp_micros())
            .bind(state.expires_at.timestamp_micros())
            .bind(i64::from(OAUTH_STATE_CAPACITY))
            .execute(&mut *tx)
            .await?
            .rows_affected();
            tx.commit().await?;
            Ok(inserted)
        }
        .await;
        // Rollback-journal readers can block COMMIT even after BEGIN acquired
        // the writer. SQLx queues rollback before releasing a failed transaction.
        let admission = match result {
            Ok(1) => OAuthStateAdmission::Created,
            Ok(_) => OAuthStateAdmission::AtCapacity,
            Err(error) if is_busy(&error) => OAuthStateAdmission::Contended,
            Err(error) => return Err(storage_error_for("plane_a_oauth_state", error)),
        };
        tracing::debug!(outcome = ?admission, "OAuth state admission completed");
        Ok(admission)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn consume_by_state(&self, state: &str) -> Result<Option<OAuthStateRow>, StorageError> {
        self.consume(state, None).await
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn consume_by_state_and_provider(
        &self,
        state: &str,
        provider: &str,
    ) -> Result<Option<OAuthStateRow>, StorageError> {
        self.consume(state, Some(provider)).await
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn cleanup_expired(&self) -> Result<u64, StorageError> {
        Ok(sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM oauth_states WHERE expires_at <= {NOW}"
        )))
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get_by_state(&self, state: &str) -> Result<Option<OAuthStateRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM oauth_states WHERE state = ?"
        )))
        .bind(state)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_state)
        .transpose()
    }
}
