//! PostgreSQL transaction owner for password signup and account-token redemption.

use sqlx::PgPool;

use crate::{
    StorageError,
    auth::{AccountLifecycle, AccountTokenOutcome, PasswordRegistration},
    sql_error::{storage_error, storage_error_for},
};

/// Account transitions on the admitted deployment pool.
#[derive(Clone)]
pub struct PgAccountLifecycle {
    pool: PgPool,
}

impl PgAccountLifecycle {
    /// Use the same admitted pool as the other account repositories.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl AccountLifecycle for PgAccountLifecycle {
    #[tracing::instrument(skip_all)]
    async fn register_password_user(
        &self,
        registration: &PasswordRegistration<'_>,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        sqlx::query(
            "INSERT INTO users
                (id, email, email_verified_at, display_name, avatar_url, password_hash,
                 created_at, last_login_at, locked_until, failed_login_count, mfa_enabled,
                 mfa_secret_envelope, version, deleted_at)
             VALUES ($1, $2, NULL, $3, NULL, $4, $5, NULL, NULL, 0, FALSE, NULL, 0, NULL)",
        )
        .bind(registration.user_id)
        .bind(registration.email)
        .bind(registration.display_name)
        .bind(registration.password_hash)
        .bind(registration.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("user", error))?;

        sqlx::query(
            "INSERT INTO verification_tokens
                (token_hash, user_id, kind, payload, created_at, expires_at, consumed_at)
             VALUES ($1, $2, 'email_verification', NULL, $3, $4, NULL)",
        )
        .bind(registration.verification_hash.as_slice())
        .bind(registration.user_id)
        .bind(registration.created_at)
        .bind(registration.verification_expires_at)
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let consumed: Option<(Vec<u8>,)> = sqlx::query_as(
            "UPDATE verification_tokens SET consumed_at = NOW()
             WHERE token_hash = $1 AND kind = 'email_verification'
               AND consumed_at IS NULL AND expires_at > NOW()
             RETURNING user_id",
        )
        .bind(token_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some((user_id,)) = consumed else {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::InvalidToken);
        };

        let updated = sqlx::query(
            "UPDATE users SET email_verified_at = NOW(), version = version + 1
             WHERE id = $1 AND deleted_at IS NULL",
        )
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
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let consumed: Option<(Vec<u8>,)> = sqlx::query_as(
            "UPDATE verification_tokens SET consumed_at = NOW()
             WHERE token_hash = $1 AND kind = 'password_reset'
               AND consumed_at IS NULL AND expires_at > NOW()
             RETURNING user_id",
        )
        .bind(token_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;
        let Some((user_id,)) = consumed else {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::InvalidToken);
        };

        let updated = sqlx::query(
            "UPDATE users SET password_hash = $2, failed_login_count = 0,
                 locked_until = NULL, version = version + 1
             WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(&user_id)
        .bind(password_hash)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if updated == 0 {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(AccountTokenOutcome::UserUnavailable);
        }

        sqlx::query(
            "UPDATE verification_tokens SET consumed_at = NOW()
             WHERE user_id = $1 AND kind = 'password_reset' AND consumed_at IS NULL",
        )
        .bind(&user_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;
        Ok(AccountTokenOutcome::Applied)
    }
}
