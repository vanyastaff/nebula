//! Postgres-backed `CredentialPersistence` impl.
//!
//! Persists [`StoredCredential`] rows in the `credentials` table created by
//! migration `0030_credentials_store.sql`. The store is deliberately thin and
//! mirrors [`SqliteCredentialPersistence`](super::sqlite::SqliteCredentialPersistence):
//!
//! - `data` is an opaque `BYTEA` — the [`EncryptionLayer`] above us serialises
//!   the AES-256-GCM envelope; we never inspect or decrypt it.
//! - `owner_id` comes only from the mandatory selector, is included in every
//!   row predicate, and overwrites the compatibility metadata stamp on write.
//! - Timestamps use native `TIMESTAMPTZ` (Postgres has a proper instant type,
//!   so the SQLite millis-INTEGER workaround is unnecessary).
//! - CAS uses a conditional owner/id/version `UPDATE` and inspects
//!   `rows_affected()` to discriminate `NotFound` from `VersionConflict`. The
//!   `u64 ↔ i64` boundary is guarded explicitly; there are no silent `as` casts.
//!
//! # Caller contract
//!
//! The caller (a composition root) is responsible for running migration 0030
//! before constructing a [`PgCredentialPersistence`].
//!
//! [`EncryptionLayer`]: crate::credential::layer::EncryptionLayer

// budget-justified: cohesive Postgres adapter for the credentials table +
// migration 0030 schema; one file mirrors the SqliteCredentialPersistence / refresh_claim
// adapter pattern (helpers + row mapping + trait impl + put dispatch).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
    CredentialWriteMode, OWNER_ID_METADATA_KEY, StoredCredential, StoredCredentialHead,
};
use serde_json::Value;
use sqlx::PgPool;

/// Postgres-backed [`CredentialPersistence`].
///
/// Wraps a `PgPool`. Cheap to clone (pool is `Arc`-backed).
/// Caller must have applied migration `0030_credentials_store.sql` to the pool.
#[derive(Clone, Debug)]
pub struct PgCredentialPersistence {
    pool: PgPool,
}

impl PgCredentialPersistence {
    /// Wrap an existing pool. Caller is responsible for running migration 0030.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn backend(message: impl Into<String>) -> CredentialPersistenceError {
    CredentialPersistenceError::Backend(Box::new(std::io::Error::other(message.into())))
}

/// Guard a `u64` version counter against i64 overflow before writing to Postgres.
fn version_to_i64(v: u64) -> Result<i64, CredentialPersistenceError> {
    i64::try_from(v).map_err(|_| backend(format!("version {v} overflows i64 (Postgres BIGINT)")))
}

/// Serialize the metadata map to a JSON string for the `TEXT` column.
fn meta_to_json(
    meta: &serde_json::Map<String, Value>,
) -> Result<String, CredentialPersistenceError> {
    serde_json::to_string(meta).map_err(|e| backend(format!("failed to serialize metadata: {e}")))
}

/// Deserialize the `TEXT` metadata column back to a map.
fn json_to_meta(s: &str) -> Result<serde_json::Map<String, Value>, CredentialPersistenceError> {
    serde_json::from_str(s).map_err(|e| backend(format!("failed to deserialize metadata: {e}")))
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

/// Projection used by management reads. Deliberately has no `data` field, so
/// sqlx cannot fetch credential material on this path.
#[derive(sqlx::FromRow)]
struct CredentialHeadRow {
    id: String,
    name: Option<String>,
    credential_key: String,
    state_kind: String,
    state_version: i64,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    reauth_required: bool,
    metadata: String,
}

impl CredentialHeadRow {
    fn into_stored_head(self) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let version = u64::try_from(self.version).map_err(|_| {
            backend(format!(
                "stored version {} is negative — table corruption",
                self.version
            ))
        })?;
        let state_version = u32::try_from(self.state_version).map_err(|_| {
            backend(format!(
                "stored state_version {} out of u32 range — table corruption",
                self.state_version
            ))
        })?;
        Ok(StoredCredentialHead {
            id: self.id,
            name: self.name,
            credential_key: self.credential_key,
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

impl CredentialRow {
    fn into_stored(self) -> Result<StoredCredential, CredentialPersistenceError> {
        let version = u64::try_from(self.version).map_err(|_| {
            backend(format!(
                "stored version {} is negative — table corruption",
                self.version
            ))
        })?;
        let state_version = u32::try_from(self.state_version).map_err(|_| {
            backend(format!(
                "stored state_version {} out of u32 range — table corruption",
                self.state_version
            ))
        })?;
        Ok(StoredCredential {
            id: self.id,
            name: self.name,
            credential_key: self.credential_key,
            data: self.data.into(),
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

// ── CredentialPersistence impl ──────────────────────────────────────────────────────

#[async_trait]
impl CredentialPersistence for PgCredentialPersistence {
    #[tracing::instrument(skip(self, selector), fields(credential.id = selector.credential_id()))]
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata \
             FROM credentials WHERE id = $1 AND owner_id = $2",
        )
        .bind(selector.credential_id())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

        match row {
            Some(r) => r.into_stored(),
            None => Err(CredentialPersistenceError::NotFound {
                credential_id: selector.credential_id().to_owned(),
            }),
        }
    }

    #[tracing::instrument(skip(self, selector), fields(credential.id = selector.credential_id()))]
    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let row: Option<CredentialHeadRow> = sqlx::query_as(
            "SELECT id, name, credential_key, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata \
             FROM credentials WHERE id = $1 AND owner_id = $2",
        )
        .bind(selector.credential_id())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

        match row {
            Some(row) => row.into_stored_head(),
            None => Err(CredentialPersistenceError::NotFound {
                credential_id: selector.credential_id().to_owned(),
            }),
        }
    }

    #[tracing::instrument(skip(self, selector, credential), fields(credential.id = selector.credential_id()))]
    async fn put(
        &self,
        selector: &CredentialSelector,
        mut credential: StoredCredential,
        mode: CredentialWriteMode,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        if credential.id != selector.credential_id() {
            return Err(CredentialPersistenceError::InvalidRequest(
                "selector credential id does not match row id",
            ));
        }
        credential.metadata.insert(
            OWNER_ID_METADATA_KEY.to_owned(),
            Value::String(selector.owner().as_str().to_owned()),
        );
        match mode {
            CredentialWriteMode::CreateOnly => self.put_create_only(selector, credential).await,
            CredentialWriteMode::Overwrite => self.put_overwrite(selector, credential).await,
            CredentialWriteMode::CompareAndSwap { expected_version } => {
                self.put_cas(selector, credential, expected_version).await
            },
            _ => Err(CredentialPersistenceError::InvalidRequest(
                "unsupported credential write mode",
            )),
        }
    }

    #[tracing::instrument(skip(self, selector), fields(credential.id = selector.credential_id()))]
    async fn delete(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), CredentialPersistenceError> {
        let result = sqlx::query("DELETE FROM credentials WHERE id = $1 AND owner_id = $2")
            .bind(selector.credential_id())
            .bind(selector.owner().as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            return Err(CredentialPersistenceError::NotFound {
                credential_id: selector.credential_id().to_owned(),
            });
        }
        Ok(())
    }

    #[tracing::instrument(skip(self, owner))]
    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<String>, CredentialPersistenceError> {
        let ids: Vec<(String,)> = match state_kind {
            Some(kind) => {
                sqlx::query_as("SELECT id FROM credentials WHERE owner_id = $1 AND state_kind = $2")
                    .bind(owner.as_str())
                    .bind(kind)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| CredentialPersistenceError::Backend(e.into()))?
            },
            None => sqlx::query_as("SELECT id FROM credentials WHERE owner_id = $1")
                .bind(owner.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(|e| CredentialPersistenceError::Backend(e.into()))?,
        };
        Ok(ids.into_iter().map(|(id,)| id).collect())
    }

    #[tracing::instrument(skip(self, owner))]
    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        let rows: Vec<CredentialHeadRow> = match state_kind {
            Some(kind) => sqlx::query_as(
                "SELECT id, name, credential_key, state_kind, state_version, version, \
                 created_at, updated_at, expires_at, reauth_required, metadata \
                 FROM credentials WHERE owner_id = $1 AND state_kind = $2 ORDER BY id",
            )
            .bind(owner.as_str())
            .bind(kind)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CredentialPersistenceError::Backend(e.into()))?,
            None => sqlx::query_as(
                "SELECT id, name, credential_key, state_kind, state_version, version, \
                 created_at, updated_at, expires_at, reauth_required, metadata \
                 FROM credentials WHERE owner_id = $1 ORDER BY id",
            )
            .bind(owner.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CredentialPersistenceError::Backend(e.into()))?,
        };
        rows.into_iter()
            .map(CredentialHeadRow::into_stored_head)
            .collect()
    }

    #[tracing::instrument(skip(self, selector), fields(credential.id = selector.credential_id()))]
    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM credentials WHERE id = $1 AND owner_id = $2 LIMIT 1")
                .bind(selector.credential_id())
                .bind(selector.owner().as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;
        Ok(row.is_some())
    }
}

// ── put dispatch helpers ──────────────────────────────────────────────────────

impl PgCredentialPersistence {
    /// `CreateOnly`: INSERT; fail with `AlreadyExists` on PRIMARY KEY conflict.
    async fn put_create_only(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let id = credential.id.clone();
        let now = Utc::now();
        let owner_id = selector.owner().as_str();
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
        .bind(owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(credential.data.as_ref())
        .bind(version_i64)
        .bind(now)
        .bind(credential.expires_at)
        .bind(credential.reauth_required)
        .bind(&meta_json)
        .execute(&self.pool)
        .await;

        match result {
            // Read back the persisted row so the caller sees the canonical form.
            Ok(_) => self.get(selector).await,
            Err(sqlx::Error::Database(db_err))
                if db_err.kind() == sqlx::error::ErrorKind::UniqueViolation =>
            {
                // Only a primary-key collision means this id already exists.
                // The `(owner_id, name)` partial-unique-index
                // (`idx_credentials_owner_name`) firing on a *new* id is a name
                // collision, not `AlreadyExists { id }` — surface it as Backend.
                match db_err.constraint() {
                    Some("credentials_pkey") => {
                        if self.exists(selector).await? {
                            Err(CredentialPersistenceError::AlreadyExists { credential_id: id })
                        } else {
                            Err(CredentialPersistenceError::NotFound { credential_id: id })
                        }
                    },
                    other => Err(backend(format!(
                        "credential unique-constraint violation ({}) — not a primary-key \
                             collision",
                        other.unwrap_or("unknown")
                    ))),
                }
            },
            Err(e) => Err(CredentialPersistenceError::Backend(e.into())),
        }
    }

    /// `Overwrite`: UPSERT; version = existing + 1 (or 1 for a new row).
    ///
    /// The read-then-update is NOT atomic under concurrent writers — but
    /// `Overwrite` does not guarantee atomicity (last-writer-wins is acceptable).
    /// Callers needing atomicity must use CAS.
    async fn put_overwrite(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let id = credential.id.clone();

        let existing_version: Option<i64> =
            sqlx::query_as("SELECT version FROM credentials WHERE id = $1 AND owner_id = $2")
                .bind(&id)
                .bind(selector.owner().as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CredentialPersistenceError::Backend(e.into()))?
                .map(|(v,): (i64,)| v);

        let Some(existing_version) = existing_version else {
            return self.put_create_only(selector, credential).await;
        };
        let new_version = version_to_i64(
            u64::try_from(existing_version)
                .map_err(|_| {
                    backend(format!(
                        "stored version {existing_version} is negative — table corruption"
                    ))
                })?
                .saturating_add(1),
        )?;

        let now = Utc::now();
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = i64::from(credential.state_version);

        let result = sqlx::query(
            "UPDATE credentials SET \
               name            = $3, \
               credential_key  = $4, \
               state_kind      = $5, \
               state_version   = $6, \
               data            = $7, \
               version         = $8, \
               updated_at      = $9, \
               expires_at      = $10, \
               reauth_required = $11, \
               metadata        = $12 \
             WHERE id = $1 AND owner_id = $2",
        )
        .bind(&credential.id)
        .bind(selector.owner().as_str())
        .bind(&credential.name)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(credential.data.as_ref())
        .bind(new_version)
        .bind(now)
        .bind(credential.expires_at)
        .bind(credential.reauth_required)
        .bind(&meta_json)
        .execute(&self.pool)
        .await
        .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            return Err(CredentialPersistenceError::NotFound { credential_id: id });
        }
        self.get(selector).await
    }

    /// `CompareAndSwap`: UPDATE WHERE version = expected; distinguish
    /// `VersionConflict` (row exists, wrong version) from `NotFound` (absent).
    async fn put_cas(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
        expected_version: u64,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let id = credential.id.clone();
        let expected_i64 = version_to_i64(expected_version)?;
        let new_version = version_to_i64(expected_version.saturating_add(1))?;
        let now = Utc::now();
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = i64::from(credential.state_version);

        let result = sqlx::query(
            "UPDATE credentials SET \
               name            = $3, \
               credential_key  = $4, \
               state_kind      = $5, \
               state_version   = $6, \
               data            = $7, \
               version         = $8, \
               updated_at      = $9, \
               expires_at      = $10, \
               reauth_required = $11, \
               metadata        = $12 \
             WHERE id = $1 AND owner_id = $2 AND version = $13",
        )
        .bind(&credential.id)
        .bind(selector.owner().as_str())
        .bind(&credential.name)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(credential.data.as_ref())
        .bind(new_version)
        .bind(now)
        .bind(credential.expires_at)
        .bind(credential.reauth_required)
        .bind(&meta_json)
        .bind(expected_i64)
        .execute(&self.pool)
        .await
        .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            // Either the row is absent or its version didn't match. Fetch to
            // distinguish the two cases for the caller.
            let current: Option<(i64,)> =
                sqlx::query_as("SELECT version FROM credentials WHERE id = $1 AND owner_id = $2")
                    .bind(&id)
                    .bind(selector.owner().as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

            return match current {
                None => Err(CredentialPersistenceError::NotFound { credential_id: id }),
                // A negative stored version is table corruption, not a normal
                // version mismatch; surface it as Backend rather than fabricating
                // an `actual` (which would mask the corruption as VersionConflict).
                Some((actual_i64,)) => match u64::try_from(actual_i64) {
                    Ok(actual) => Err(CredentialPersistenceError::VersionConflict {
                        credential_id: id,
                        expected: expected_version,
                        actual,
                    }),
                    Err(_) => Err(backend(format!(
                        "stored version {actual_i64} is negative — table corruption"
                    ))),
                },
            };
        }

        self.get(selector).await
    }
}
