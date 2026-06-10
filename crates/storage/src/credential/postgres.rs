//! Postgres-backed `CredentialStore` impl.
//!
//! Persists [`StoredCredential`] rows in the `credentials` table created by
//! migration `0030_credentials_store.sql`. The store is deliberately thin and
//! mirrors [`SqliteCredentialStore`](super::sqlite::SqliteCredentialStore):
//!
//! - `data` is an opaque `BYTEA` — the [`EncryptionLayer`] above us serialises
//!   the AES-256-GCM envelope; we never inspect or decrypt it.
//! - `owner_id` is extracted from `metadata["owner_id"]` at write time and
//!   stored in its own column so the partial-unique-name index is queryable.
//! - Timestamps use native `TIMESTAMPTZ` (Postgres has a proper instant type,
//!   so the SQLite millis-INTEGER workaround is unnecessary).
//! - CAS uses a conditional `UPDATE … WHERE version = $13` and inspects
//!   `rows_affected()` to discriminate `NotFound` from `VersionConflict`. The
//!   `u64 ↔ i64` boundary is guarded explicitly; there are no silent `as` casts.
//!
//! # Caller contract
//!
//! The caller (a composition root) is responsible for running migration 0030
//! before constructing a [`PgCredentialStore`].
//!
//! [`EncryptionLayer`]: crate::credential::layer::EncryptionLayer

// budget-justified: cohesive Postgres adapter for the credentials table +
// migration 0030 schema; one file mirrors the SqliteCredentialStore / refresh_claim
// adapter pattern (helpers + row mapping + trait impl + put dispatch).

use chrono::{DateTime, Utc};
use nebula_credential::{CredentialStore, PutMode, StoreError, StoredCredential};
use serde_json::Value;
use sqlx::PgPool;

/// Postgres-backed [`CredentialStore`].
///
/// Wraps a `PgPool`. Cheap to clone (pool is `Arc`-backed).
/// Caller must have applied migration `0030_credentials_store.sql` to the pool.
#[derive(Clone, Debug)]
pub struct PgCredentialStore {
    pool: PgPool,
}

impl PgCredentialStore {
    /// Wrap an existing pool. Caller is responsible for running migration 0030.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Guard a `u64` version counter against i64 overflow before writing to Postgres.
fn version_to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| {
        StoreError::Backend(format!("version {v} overflows i64 (Postgres BIGINT)").into())
    })
}

/// Extract `owner_id` from the metadata map if present.
fn owner_id_from_metadata(meta: &serde_json::Map<String, Value>) -> Option<&str> {
    meta.get("owner_id").and_then(|v| v.as_str())
}

/// Serialize the metadata map to a JSON string for the `TEXT` column.
fn meta_to_json(meta: &serde_json::Map<String, Value>) -> Result<String, StoreError> {
    serde_json::to_string(meta)
        .map_err(|e| StoreError::Backend(format!("failed to serialize metadata: {e}").into()))
}

/// Deserialize the `TEXT` metadata column back to a map.
fn json_to_meta(s: &str) -> Result<serde_json::Map<String, Value>, StoreError> {
    serde_json::from_str(s)
        .map_err(|e| StoreError::Backend(format!("failed to deserialize metadata: {e}").into()))
}

// ── raw row type returned by SELECT queries ───────────────────────────────────

/// Flat projection of a `credentials` row. `sqlx::FromRow` matches by column
/// name; every SELECT in this file lists the same columns.
#[derive(sqlx::FromRow)]
struct CredentialRow {
    id: String,
    name: Option<String>,
    credential_key: String,
    data: Vec<u8>,
    state_kind: String,
    state_version: i64,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    reauth_required: bool,
    metadata: String,
}

impl CredentialRow {
    fn into_stored(self) -> Result<StoredCredential, StoreError> {
        let version = u64::try_from(self.version).map_err(|_| {
            StoreError::Backend(
                format!(
                    "stored version {} is negative — table corruption",
                    self.version
                )
                .into(),
            )
        })?;
        let state_version = u32::try_from(self.state_version).map_err(|_| {
            StoreError::Backend(
                format!(
                    "stored state_version {} out of u32 range — table corruption",
                    self.state_version
                )
                .into(),
            )
        })?;
        Ok(StoredCredential {
            id: self.id,
            name: self.name,
            credential_key: self.credential_key,
            data: self.data,
            state_kind: self.state_kind,
            state_version,
            version,
            created_at: self.created_at,
            updated_at: self.updated_at,
            expires_at: self.expires_at,
            reauth_required: self.reauth_required,
            metadata: json_to_meta(&self.metadata)?,
        })
    }
}

// ── CredentialStore impl ──────────────────────────────────────────────────────

impl CredentialStore for PgCredentialStore {
    #[tracing::instrument(skip(self), fields(credential.id = id))]
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata \
             FROM credentials WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.into()))?;

        match row {
            Some(r) => r.into_stored(),
            None => Err(StoreError::NotFound { id: id.to_owned() }),
        }
    }

    #[tracing::instrument(skip(self), fields(credential.id = credential.id))]
    async fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        match mode {
            PutMode::CreateOnly => self.put_create_only(credential).await,
            PutMode::Overwrite => self.put_overwrite(credential).await,
            PutMode::CompareAndSwap { expected_version } => {
                self.put_cas(credential, expected_version).await
            },
            _ => Err(StoreError::Backend(
                format!("postgres store: unsupported PutMode variant `{mode:?}`").into(),
            )),
        }
    }

    #[tracing::instrument(skip(self), fields(credential.id = id))]
    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let result = sqlx::query("DELETE FROM credentials WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound { id: id.to_owned() });
        }
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let ids: Vec<(String,)> = match state_kind {
            Some(kind) => sqlx::query_as("SELECT id FROM credentials WHERE state_kind = $1")
                .bind(kind)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.into()))?,
            None => sqlx::query_as("SELECT id FROM credentials")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.into()))?,
        };
        Ok(ids.into_iter().map(|(id,)| id).collect())
    }

    #[tracing::instrument(skip(self), fields(credential.id = id))]
    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let row: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM credentials WHERE id = $1 LIMIT 1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.into()))?;
        Ok(row.is_some())
    }
}

// ── put dispatch helpers ──────────────────────────────────────────────────────

impl PgCredentialStore {
    /// `CreateOnly`: INSERT; fail with `AlreadyExists` on PRIMARY KEY conflict.
    async fn put_create_only(
        &self,
        credential: StoredCredential,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        let now = Utc::now();
        let owner_id = owner_id_from_metadata(&credential.metadata).map(ToOwned::to_owned);
        let meta_json = meta_to_json(&credential.metadata)?;
        let version_i64: i64 = 1;
        let state_version_i64 = i64::from(credential.state_version);

        let result = sqlx::query(
            "INSERT INTO credentials \
             (id, name, owner_id, credential_key, state_kind, state_version, \
              data, version, created_at, updated_at, expires_at, \
              reauth_required, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $10, $11, $12)",
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(&credential.data)
        .bind(version_i64)
        .bind(now)
        .bind(credential.expires_at)
        .bind(credential.reauth_required)
        .bind(&meta_json)
        .execute(&self.pool)
        .await;

        match result {
            // Read back the persisted row so the caller sees the canonical form.
            Ok(_) => self.get(&id).await,
            Err(sqlx::Error::Database(db_err))
                if db_err.kind() == sqlx::error::ErrorKind::UniqueViolation =>
            {
                Err(StoreError::AlreadyExists { id })
            },
            Err(e) => Err(StoreError::Backend(e.into())),
        }
    }

    /// `Overwrite`: UPSERT; version = existing + 1 (or 1 for a new row).
    ///
    /// The read-then-update is NOT atomic under concurrent writers — but
    /// `Overwrite` does not guarantee atomicity (last-writer-wins is acceptable).
    /// Callers needing atomicity must use CAS.
    async fn put_overwrite(
        &self,
        credential: StoredCredential,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();

        let existing_version: Option<i64> =
            sqlx::query_as("SELECT version FROM credentials WHERE id = $1")
                .bind(&id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.into()))?
                .map(|(v,): (i64,)| v);

        let new_version: i64 = match existing_version {
            Some(v) => {
                let v_u64 = u64::try_from(v).map_err(|_| {
                    StoreError::Backend(
                        format!("stored version {v} is negative — table corruption").into(),
                    )
                })?;
                version_to_i64(v_u64.saturating_add(1))?
            },
            None => 1,
        };

        let now = Utc::now();
        let owner_id = owner_id_from_metadata(&credential.metadata).map(ToOwned::to_owned);
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = i64::from(credential.state_version);

        // `created_at` is preserved on conflict (only set on insert via $9).
        sqlx::query(
            "INSERT INTO credentials \
             (id, name, owner_id, credential_key, state_kind, state_version, \
              data, version, created_at, updated_at, expires_at, \
              reauth_required, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $10, $11, $12) \
             ON CONFLICT (id) DO UPDATE SET \
               name            = EXCLUDED.name, \
               owner_id        = EXCLUDED.owner_id, \
               credential_key  = EXCLUDED.credential_key, \
               state_kind      = EXCLUDED.state_kind, \
               state_version   = EXCLUDED.state_version, \
               data            = EXCLUDED.data, \
               version         = $8, \
               updated_at      = $9, \
               expires_at      = EXCLUDED.expires_at, \
               reauth_required = EXCLUDED.reauth_required, \
               metadata        = EXCLUDED.metadata",
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(&credential.data)
        .bind(new_version)
        .bind(now)
        .bind(credential.expires_at)
        .bind(credential.reauth_required)
        .bind(&meta_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.into()))?;

        self.get(&id).await
    }

    /// `CompareAndSwap`: UPDATE WHERE version = expected; distinguish
    /// `VersionConflict` (row exists, wrong version) from `NotFound` (absent).
    async fn put_cas(
        &self,
        credential: StoredCredential,
        expected_version: u64,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        let expected_i64 = version_to_i64(expected_version)?;
        let new_version = version_to_i64(expected_version.saturating_add(1))?;
        let now = Utc::now();
        let owner_id = owner_id_from_metadata(&credential.metadata).map(ToOwned::to_owned);
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = i64::from(credential.state_version);

        let result = sqlx::query(
            "UPDATE credentials SET \
               name            = $2, \
               owner_id        = $3, \
               credential_key  = $4, \
               state_kind      = $5, \
               state_version   = $6, \
               data            = $7, \
               version         = $8, \
               updated_at      = $9, \
               expires_at      = $10, \
               reauth_required = $11, \
               metadata        = $12 \
             WHERE id = $1 AND version = $13",
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(&credential.data)
        .bind(new_version)
        .bind(now)
        .bind(credential.expires_at)
        .bind(credential.reauth_required)
        .bind(&meta_json)
        .bind(expected_i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            // Either the row is absent or its version didn't match. Fetch to
            // distinguish the two cases for the caller.
            let current: Option<(i64,)> =
                sqlx::query_as("SELECT version FROM credentials WHERE id = $1")
                    .bind(&id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| StoreError::Backend(e.into()))?;

            return match current {
                None => Err(StoreError::NotFound { id }),
                Some((actual_i64,)) => {
                    let actual = u64::try_from(actual_i64).unwrap_or(u64::MAX);
                    Err(StoreError::VersionConflict {
                        id,
                        expected: expected_version,
                        actual,
                    })
                },
            };
        }

        self.get(&id).await
    }
}
