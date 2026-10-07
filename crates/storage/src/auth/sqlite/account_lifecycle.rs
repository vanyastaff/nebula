//! Atomic account transitions under SQLite's database-wide writer lock.

use sqlx::SqlitePool;

use super::NOW;
use crate::{
    StorageError,
    auth::{AccountLifecycle, AccountTokenOutcome, PasswordRegistration},
    sql_error::{storage_error, storage_error_for},
};

/// SQLite owner of registration, verification and password-reset transactions.
#[derive(Clone)]
pub struct SqliteAccountLifecycle {
    pool: SqlitePool,
}

impl SqliteAccountLifecycle {
    /// Bind to the application's admitted pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl AccountLifecycle for SqliteAccountLifecycle {
    #[tracing::instrument(skip_all)]
    async fn register_password_user(
        &self,
        registration: &PasswordRegistration<'_>,
    ) -> Result<(), StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        sqlx::query(
            "INSERT INTO users
            (id, email, display_name, password_hash, created_at)
            VALUES (?, ?, ?, ?, ?)",
        )
        .bind(registration.user_id)
        .bind(registration.email)
        .bind(registration.display_name)
        .bind(registration.password_hash)
        .bind(registration.created_at.timestamp_micros())
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("user", error))?;
        sqlx::query(
            "INSERT INTO verification_tokens
            (token_hash, user_id, kind, created_at, expires_at)
            VALUES (?, ?, 'email_verification', ?, ?)",
        )
        .bind(registration.verification_hash.as_slice())
        .bind(registration.user_id)
        .bind(registration.created_at.timestamp_micros())
        .bind(registration.verification_expires_at.timestamp_micros())
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("verification_token", error))?;
        tx.commit().await.map_err(storage_error)
    }

    #[tracing::instrument(skip_all)]
    async fn verify_email(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<AccountTokenOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let consumed: Option<Vec<u8>> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "UPDATE verification_tokens SET consumed_at = {NOW}
             WHERE token_hash = ? AND kind = 'email_verification'
               AND consumed_at IS NULL AND expires_at > {NOW}
             RETURNING user_id"
        )))
        .bind(token_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some(user_id) = consumed else {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::InvalidToken);
        };
        let updated = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE users SET email_verified_at = {NOW}, version = version + 1
             WHERE id = ? AND deleted_at IS NULL"
        )))
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if updated == 0 {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::UserUnavailable);
        }
        tx.commit().await.map_err(storage_error)?;
        Ok(AccountTokenOutcome::Applied)
    }

    #[tracing::instrument(skip_all)]
    async fn reset_password(
        &self,
        token_hash: &[u8; 32],
        password_hash: &str,
    ) -> Result<AccountTokenOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let consumed: Option<Vec<u8>> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "UPDATE verification_tokens SET consumed_at = {NOW}
             WHERE token_hash = ? AND kind = 'password_reset'
               AND consumed_at IS NULL AND expires_at > {NOW}
             RETURNING user_id"
        )))
        .bind(token_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some(user_id) = consumed else {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::InvalidToken);
        };
        let updated = sqlx::query(
            "UPDATE users SET password_hash = ?, failed_login_count = 0,
             locked_until = NULL, version = version + 1
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(password_hash)
        .bind(&user_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if updated == 0 {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::UserUnavailable);
        }
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE verification_tokens SET consumed_at = {NOW}
             WHERE user_id = ? AND kind = 'password_reset' AND consumed_at IS NULL"
        )))
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(AccountTokenOutcome::Applied)
    }
}
