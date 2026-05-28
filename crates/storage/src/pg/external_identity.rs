//! Postgres implementation of [`ExternalIdentityRepo`].
//!
//! Schema: migration `0021_external_identities.sql`. Holds the
//! Plane-A `(provider, subject) → user_id` linkage per ADR-0085 D-8.
//!
//! # Atomicity
//!
//! - [`link_external`](ExternalIdentityRepo::link_external) is a plain
//!   `INSERT`. The `(provider, subject)` PK rejects duplicates with
//!   SQLSTATE 23505 → `StorageError::Duplicate`; callers race only on
//!   the first-login path and the loser typically retries the read.
//! - [`find_user_by_external`](ExternalIdentityRepo::find_user_by_external)
//!   is a single-row `SELECT` keyed by the PK; consistent-read by
//!   default.
//!
//! # Cascade
//!
//! `user_id` has `ON DELETE CASCADE` per the migration. Deleting a
//! user atomically purges every external identity link for them. The
//! repo does not need to model this — Postgres enforces it.

use sqlx::{Pool, Postgres};

use crate::{error::StorageError, pg::map_db_err, repos::ExternalIdentityRepo};

/// Postgres-backed `external_identities` repository.
#[derive(Clone)]
pub struct PgExternalIdentityRepo {
    pool: Pool<Postgres>,
}

impl PgExternalIdentityRepo {
    /// Construct from an existing pool.
    #[must_use]
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }
}

impl ExternalIdentityRepo for PgExternalIdentityRepo {
    #[tracing::instrument(level = "debug", skip(self, subject), fields(provider))]
    async fn find_user_by_external(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let row: Option<(Vec<u8>,)> = sqlx::query_as(
            "SELECT user_id FROM external_identities \
             WHERE provider = $1 AND subject = $2 \
             LIMIT 1",
        )
        .bind(provider)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_db_err("external_identity", e))?;
        Ok(row.map(|(user_id,)| user_id))
    }

    #[tracing::instrument(level = "debug", skip(self, user_id, subject, email), fields(provider))]
    async fn link_external(
        &self,
        user_id: &[u8],
        provider: &str,
        subject: &str,
        email: Option<&str>,
    ) -> Result<(), StorageError> {
        // Plain INSERT; PK rejects duplicate (provider, subject) with
        // SQLSTATE 23505 → StorageError::Duplicate (caller decides
        // whether to treat that as race-loss or as a user-pointed
        // already-linked error).
        sqlx::query(
            "INSERT INTO external_identities (provider, subject, user_id, email) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(provider)
        .bind(subject)
        .bind(user_id)
        .bind(email)
        .execute(&self.pool)
        .await
        .map_err(|e| map_db_err("external_identity", e))?;
        Ok(())
    }
}
