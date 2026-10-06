//! `port_users`: global accounts; email is unique among active rows
//! (case-insensitive, via a `lower(email)` partial unique index).

use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::UserRow;
use nebula_storage_port::store::UserStore;
use sqlx::PgPool;
use sqlx::postgres::PgRow;

use super::{
    cas_disambiguate, encode_version, int32, optional, required, soft_delete_by_id, version,
};
use crate::sql_error::{storage_error, storage_error_for};

/// Postgres-backed `users` store.
#[derive(Clone, Debug)]
pub struct PgUserStore {
    pool: PgPool,
}

impl PgUserStore {
    /// Wrap a pool whose schema was installed via [`crate::postgres::init_schema`].
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn decode_user(row: &PgRow) -> Result<Arc<UserRow>, StorageError> {
    Ok(Arc::new(UserRow {
        id: required(row, "id")?,
        email: required(row, "email")?,
        email_verified_at: optional(row, "email_verified_at")?,
        display_name: required(row, "display_name")?,
        avatar_url: optional(row, "avatar_url")?,
        password_hash: optional(row, "password_hash")?,
        created_at: required(row, "created_at")?,
        last_login_at: optional(row, "last_login_at")?,
        locked_until: optional(row, "locked_until")?,
        failed_login_count: int32(row, "failed_login_count")?,
        mfa_enabled: required(row, "mfa_enabled")?,
        // The schema column is `mfa_secret`; `mfa_secret_envelope` is the
        // domain field name.
        mfa_secret_envelope: optional(row, "mfa_secret")?,
        version: version(row)?,
        deleted_at: optional(row, "deleted_at")?,
    }))
}

#[async_trait::async_trait]
impl UserStore for PgUserStore {
    async fn create(&self, row: UserRow) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO port_users (id, email, email_verified_at, display_name, \
             avatar_url, password_hash, created_at, last_login_at, locked_until, \
             failed_login_count, mfa_enabled, mfa_secret, version, deleted_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(&row.id)
        .bind(&row.email)
        .bind(&row.email_verified_at)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(&row.password_hash)
        .bind(&row.created_at)
        .bind(&row.last_login_at)
        .bind(&row.locked_until)
        .bind(i64::from(row.failed_login_count))
        .bind(row.mfa_enabled)
        .bind(&row.mfa_secret_envelope)
        .bind(encode_version(row.version)?)
        .bind(&row.deleted_at)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("user", error))?;
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        sqlx::query("SELECT * FROM port_users WHERE id = $1 AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage_error)?
            .as_ref()
            .map(decode_user)
            .transpose()
    }

    async fn get_by_email(&self, email: &str) -> Result<Option<Arc<UserRow>>, StorageError> {
        sqlx::query(
            "SELECT * FROM port_users \
             WHERE lower(email) = lower($1) AND deleted_at IS NULL",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .as_ref()
        .map(decode_user)
        .transpose()
    }

    async fn update(&self, row: UserRow, expected_version: u64) -> Result<(), StorageError> {
        let res = sqlx::query(
            "UPDATE port_users SET email = $1, email_verified_at = $2, \
             display_name = $3, avatar_url = $4, password_hash = $5, \
             last_login_at = $6, locked_until = $7, failed_login_count = $8, \
             mfa_enabled = $9, mfa_secret = $10, version = $11 \
             WHERE id = $12 AND deleted_at IS NULL AND version = $13",
        )
        .bind(&row.email)
        .bind(&row.email_verified_at)
        .bind(&row.display_name)
        .bind(&row.avatar_url)
        .bind(&row.password_hash)
        .bind(&row.last_login_at)
        .bind(&row.locked_until)
        .bind(i64::from(row.failed_login_count))
        .bind(row.mfa_enabled)
        .bind(&row.mfa_secret_envelope)
        .bind(encode_version(row.version)?)
        .bind(&row.id)
        .bind(encode_version(expected_version)?)
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("user", error))?;
        if res.rows_affected() > 0 {
            return Ok(());
        }
        cas_disambiguate(&self.pool, "port_users", "user", &row.id, expected_version).await
    }

    async fn soft_delete(&self, id: &str) -> Result<(), StorageError> {
        soft_delete_by_id(&self.pool, "port_users", "user", id).await
    }
}
