//! SQLite user records, versioned updates and atomic login bookkeeping.

use sqlx::{Row, SqlitePool, sqlite::SqliteRow};

use super::{NOW, instant, micros, optional_instant};
use crate::{
    StorageError,
    auth::{UserRepo, UserRow},
    sql_error::{decode_u64, storage_error, storage_error_for},
};

/// SQLite user repository on an existing deployment pool.
#[derive(Clone)]
pub struct SqliteUserRepo {
    pool: SqlitePool,
}

impl SqliteUserRepo {
    /// Bind to the application's admitted pool without opening another database.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) const COLUMNS: &str = "id, email, email_verified_at, display_name, avatar_url,
    password_hash, created_at, last_login_at, locked_until, failed_login_count,
    mfa_enabled, mfa_secret_envelope, version, deleted_at";

pub(super) fn decode_user(row: SqliteRow) -> Result<UserRow, StorageError> {
    let enabled: i64 = row.try_get("mfa_enabled").map_err(storage_error)?;
    let mfa_enabled = match enabled {
        0 => false,
        1 => true,
        _ => return Err(StorageError::Corrupt("invalid user MFA flag".into())),
    };
    Ok(UserRow {
        id: row.try_get("id").map_err(storage_error)?,
        email: row.try_get("email").map_err(storage_error)?,
        email_verified_at: optional_instant(&row, "email_verified_at")?,
        display_name: row.try_get("display_name").map_err(storage_error)?,
        avatar_url: row.try_get("avatar_url").map_err(storage_error)?,
        password_hash: row.try_get("password_hash").map_err(storage_error)?,
        created_at: instant(&row, "created_at")?,
        last_login_at: optional_instant(&row, "last_login_at")?,
        locked_until: optional_instant(&row, "locked_until")?,
        failed_login_count: row.try_get("failed_login_count").map_err(storage_error)?,
        mfa_enabled,
        mfa_secret_envelope: row.try_get("mfa_secret_envelope").map_err(storage_error)?,
        version: row.try_get("version").map_err(storage_error)?,
        deleted_at: optional_instant(&row, "deleted_at")?,
    })
}

#[async_trait::async_trait]
impl UserRepo for SqliteUserRepo {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn create(&self, user: &UserRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO users
            (id, email, email_verified_at, display_name, avatar_url, password_hash,
             created_at, last_login_at, locked_until, failed_login_count, mfa_enabled,
             mfa_secret_envelope, version, deleted_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(micros(user.email_verified_at))
        .bind(&user.display_name)
        .bind(&user.avatar_url)
        .bind(&user.password_hash)
        .bind(user.created_at.timestamp_micros())
        .bind(micros(user.last_login_at))
        .bind(micros(user.locked_until))
        .bind(user.failed_login_count)
        .bind(user.mfa_enabled)
        .bind(&user.mfa_secret_envelope)
        .bind(user.version)
        .bind(micros(user.deleted_at))
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("user", error))?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get(&self, id: &[u8]) -> Result<Option<UserRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM users WHERE id = ? AND deleted_at IS NULL"
        )))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_user)
        .transpose()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get_by_email(&self, email: &str) -> Result<Option<UserRow>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM users WHERE lower(email) = lower(?) AND deleted_at IS NULL"
        )))
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_user)
        .transpose()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn update(&self, user: &UserRow, expected_version: i64) -> Result<(), StorageError> {
        let expected = u64::try_from(expected_version)
            .map_err(|_| StorageError::InvalidInput("user version is negative".into()))?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let updated = sqlx::query(
            "UPDATE users SET
            email = ?, email_verified_at = ?, display_name = ?, avatar_url = ?, password_hash = ?,
            last_login_at = ?, locked_until = ?, failed_login_count = ?, mfa_enabled = ?,
            mfa_secret_envelope = ?, version = version + 1
            WHERE id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(&user.email)
        .bind(micros(user.email_verified_at))
        .bind(&user.display_name)
        .bind(&user.avatar_url)
        .bind(&user.password_hash)
        .bind(micros(user.last_login_at))
        .bind(micros(user.locked_until))
        .bind(user.failed_login_count)
        .bind(user.mfa_enabled)
        .bind(&user.mfa_secret_envelope)
        .bind(&user.id)
        .bind(expected_version)
        .execute(&mut *tx)
        .await
        .map_err(|error| storage_error_for("user", error))?
        .rows_affected();
        if updated == 0 {
            let actual: Option<i64> =
                sqlx::query_scalar("SELECT version FROM users WHERE id = ? AND deleted_at IS NULL")
                    .bind(&user.id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(storage_error)?;
            tx.rollback().await.map_err(storage_error)?;
            return Err(match actual {
                Some(actual) => StorageError::Conflict {
                    entity: "user",
                    id: hex::encode(&user.id),
                    expected,
                    actual: decode_u64(actual, "version")?,
                },
                None => StorageError::not_found("user", hex::encode(&user.id)),
            });
        }
        tx.commit().await.map_err(storage_error)
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn soft_delete(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE users SET deleted_at = {NOW}, version = version + 1
             WHERE id = ? AND deleted_at IS NULL"
        )))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn record_login_success(&self, id: &[u8]) -> Result<(), StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE users SET last_login_at = {NOW}, failed_login_count = 0, locked_until = NULL
             WHERE id = ? AND deleted_at IS NULL"
        )))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn record_login_failure(&self, id: &[u8]) -> Result<(), StorageError> {
        // One write serializes increments across independent connections. Login
        // bookkeeping deliberately leaves the profile CAS version unchanged.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE users SET
            failed_login_count = failed_login_count + 1,
            locked_until = CASE WHEN failed_login_count + 1 >= 5 THEN {NOW} + 900000000 ELSE locked_until END
            WHERE id = ? AND deleted_at IS NULL"
        )))
            .bind(id).execute(&self.pool).await.map_err(storage_error)?;
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn rotate_mfa_secret_envelope(
        &self,
        user_id: &[u8],
        expected_envelope: &[u8],
        replacement_envelope: &[u8],
    ) -> Result<bool, StorageError> {
        Ok(sqlx::query(
            "UPDATE users SET mfa_secret_envelope = ?, version = version + 1
            WHERE id = ? AND mfa_secret_envelope = ? AND deleted_at IS NULL",
        )
        .bind(replacement_envelope)
        .bind(user_id)
        .bind(expected_envelope)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?
        .rows_affected()
            == 1)
    }
}
