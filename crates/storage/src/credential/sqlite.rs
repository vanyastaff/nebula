//! SQLite-backed `CredentialStore` impl.
//!
//! Persists [`StoredCredential`] rows in the `credentials` table created by
//! migration `0030_credentials_store.sql`. The store is deliberately thin:
//!
//! - `data` is an opaque `BLOB` — the [`EncryptionLayer`] above us serialises
//!   the AES-256-GCM envelope; we never inspect or decrypt it.
//! - `owner_id` is extracted from `metadata["owner_id"]` at write time and
//!   stored in its own column so the partial-unique-name index is queryable.
//! - Timestamps are stored as `INTEGER` milliseconds-since-epoch (UTC), not
//!   RFC-3339 text, for the same reasons documented in the `RefreshClaimRepo`
//!   SQLite impl (`refresh_claim/sqlite.rs`): integer ordering is unambiguous
//!   for expiry predicates across chrono versions.
//! - CAS uses a conditional `UPDATE … WHERE version = :expected_version` and
//!   inspects `rows_affected()` to discriminate between `NotFound` and
//!   `VersionConflict`. The `u64 ↔ i64` boundary is guarded explicitly;
//!   there are no silent `as` casts.
//!
//! # Caller contract
//!
//! The caller (a composition root) is responsible for running migration 0030
//! before constructing a [`SqliteCredentialStore`].
//!
//! [`EncryptionLayer`]: crate::credential::layer::EncryptionLayer

// budget-justified: cohesive SQLite adapter for the credentials table +
// migration 0030 schema; one file mirrors the established refresh_claim/sqlite.rs
// adapter pattern (helpers + row mapping + trait impl + put dispatch).

use chrono::{DateTime, TimeZone, Utc};
use nebula_credential::{CredentialStore, PutMode, StoreError, StoredCredential};
use serde_json::Value;
use sqlx::SqlitePool;

/// SQLite-backed [`CredentialStore`].
///
/// Wraps a `SqlitePool`. Cheap to clone (pool is `Arc`-backed).
/// Caller must have applied migration `0030_credentials_store.sql` to the pool.
#[derive(Clone, Debug)]
pub struct SqliteCredentialStore {
    pool: SqlitePool,
}

impl SqliteCredentialStore {
    /// Wrap an existing pool. Caller is responsible for running migration 0030.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Open (creating if absent) a SQLite database at `url` and apply migration
    /// `0030_credentials_store.sql`, returning a ready store.
    ///
    /// `url` is a SQLite connection string — a file URL
    /// (`sqlite://path/to/credentials.db`), a bare path, or
    /// `sqlite::memory:` for an ephemeral store. The database is expected to be
    /// credential-dedicated: migration 0030 creates a self-contained
    /// `credentials` table with no foreign keys, so the full migration chain is
    /// not required.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] if the URL is malformed, the database
    /// cannot be opened/created, or the migration fails to apply.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        use std::str::FromStr;

        let options = sqlx::sqlite::SqliteConnectOptions::from_str(url)
            .map_err(|e| StoreError::Backend(format!("invalid SQLite URL `{url}`: {e}").into()))?
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(|e| StoreError::Backend(format!("open SQLite `{url}`: {e}").into()))?;

        // Bootstrap is idempotent: migration 0030 begins with `DROP TABLE`
        // (it removes the legacy Model-B schema), so re-running it on every
        // connect would WIPE a populated store on each restart — defeating
        // durability. Apply it only when the `credentials` table is absent
        // (a fresh database); an already-provisioned store keeps its rows.
        let provisioned: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'credentials'",
        )
        .fetch_optional(&pool)
        .await
        .map_err(|e| StoreError::Backend(format!("probe credentials table: {e}").into()))?;
        if provisioned.is_none() {
            sqlx::query(include_str!(
                "../../migrations/sqlite/0030_credentials_store.sql"
            ))
            .execute(&pool)
            .await
            .map_err(|e| StoreError::Backend(format!("apply migration 0030: {e}").into()))?;
        }

        Ok(Self { pool })
    }

    /// Open a fresh, uniquely-named in-memory SQLite store with migration 0030
    /// applied — the standard test backend for credential-store consumers.
    ///
    /// Each call gets an isolated `mode=memory&cache=shared` database keyed by a
    /// random name, so concurrent tests in the same process never collide and a
    /// pool with multiple connections all observe the same data (plain
    /// `sqlite::memory:` gives each connection a private, invisible database).
    /// A shared-cache in-memory database is destroyed when its **last**
    /// connection closes, so the pool pins one live connection
    /// (`min_connections(1)`) for its lifetime; the database survives idle gaps
    /// between operations and dies only when the store (and its pool) is
    /// dropped. Hold the returned store for the lifetime of the test.
    ///
    /// Test-only (`test-util` feature / `cfg(test)`); never compiled into a
    /// release build (ADR-0023).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] if the pool cannot be opened or migration
    /// 0030 fails to apply.
    #[cfg(any(test, feature = "test-util"))]
    pub async fn connect_memory() -> Result<Self, StoreError> {
        use std::str::FromStr;

        let name = uuid::Uuid::new_v4();
        let url = format!("sqlite:file:nebula-cred-mem-{name}?mode=memory&cache=shared");
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
            .map_err(|e| StoreError::Backend(format!("invalid SQLite URL `{url}`: {e}").into()))?
            .create_if_missing(true);
        // `min_connections(1)` keeps one connection open for the pool's lifetime
        // so the shared-cache in-memory database is not destroyed during idle
        // gaps; `max_connections(4)` lets the concurrency tests exercise real
        // SQL-layer contention.
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(|e| {
                StoreError::Backend(format!("open in-memory SQLite `{url}`: {e}").into())
            })?;

        sqlx::query(include_str!(
            "../../migrations/sqlite/0030_credentials_store.sql"
        ))
        .execute(&pool)
        .await
        .map_err(|e| StoreError::Backend(format!("apply migration 0030: {e}").into()))?;

        Ok(Self { pool })
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Convert a millisecond-since-epoch `INTEGER` column back to `DateTime<Utc>`.
///
/// Mirrors the identical helper in `refresh_claim/sqlite.rs`. An out-of-range
/// value indicates table corruption; surfaced as `StoreError::Backend`.
fn millis_to_utc(ms: i64, col: &'static str) -> Result<DateTime<Utc>, StoreError> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| StoreError::Backend(format!("{col} millis out of range: {ms}").into()))
}

/// Guard a `u64` version counter against i64 overflow before writing to SQLite.
fn version_to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(|_| {
        StoreError::Backend(format!("version {v} overflows i64 (SQLite INTEGER)").into())
    })
}

/// Guard a `u64` state_version (u32 in the DTO but stored as INTEGER) cast.
fn state_version_to_i64(v: u32) -> i64 {
    i64::from(v)
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

/// Flat projection of a `credentials` row.
///
/// `sqlx::FromRow` is derived so `query_as` can bind columns by position in
/// the SELECT list. The order must match every SELECT in this file.
#[derive(sqlx::FromRow)]
struct CredentialRow {
    id: String,
    name: Option<String>,
    credential_key: String,
    data: Vec<u8>,
    state_kind: String,
    state_version: i64,
    version: i64,
    created_at: i64,
    updated_at: i64,
    expires_at: Option<i64>,
    reauth_required: i64,
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
            created_at: millis_to_utc(self.created_at, "created_at")?,
            updated_at: millis_to_utc(self.updated_at, "updated_at")?,
            expires_at: self
                .expires_at
                .map(|ms| millis_to_utc(ms, "expires_at"))
                .transpose()?,
            reauth_required: self.reauth_required != 0,
            metadata: json_to_meta(&self.metadata)?,
        })
    }
}

// ── CredentialStore impl ──────────────────────────────────────────────────────

impl CredentialStore for SqliteCredentialStore {
    #[tracing::instrument(skip(self), fields(credential.id = id))]
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata \
             FROM credentials WHERE id = ?1",
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
                format!("sqlite store: unsupported PutMode variant `{mode:?}`").into(),
            )),
        }
    }

    #[tracing::instrument(skip(self), fields(credential.id = id))]
    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let result = sqlx::query("DELETE FROM credentials WHERE id = ?1")
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
            Some(kind) => sqlx::query_as("SELECT id FROM credentials WHERE state_kind = ?1")
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
        let row: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM credentials WHERE id = ?1 LIMIT 1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.into()))?;
        Ok(row.is_some())
    }
}

// ── put dispatch helpers ──────────────────────────────────────────────────────

impl SqliteCredentialStore {
    /// `CreateOnly`: INSERT; fail with `AlreadyExists` on PRIMARY KEY conflict.
    async fn put_create_only(
        &self,
        credential: StoredCredential,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();
        let now = Utc::now();
        let created_ms = now.timestamp_millis();
        let expires_ms: Option<i64> = credential.expires_at.map(|dt| dt.timestamp_millis());
        let owner_id = owner_id_from_metadata(&credential.metadata).map(ToOwned::to_owned);
        let meta_json = meta_to_json(&credential.metadata)?;
        let version_i64: i64 = 1;
        let state_version_i64 = state_version_to_i64(credential.state_version);
        let reauth_i64: i64 = i64::from(credential.reauth_required);

        let result = sqlx::query(
            "INSERT INTO credentials \
             (id, name, owner_id, credential_key, state_kind, state_version, \
              data, version, created_at, updated_at, expires_at, \
              reauth_required, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11, ?12)",
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(&credential.data)
        .bind(version_i64)
        .bind(created_ms)
        .bind(expires_ms)
        .bind(reauth_i64)
        .bind(&meta_json)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => {
                // Return a StoredCredential with store-assigned timestamps.
                // We read back the row rather than reconstructing from locals
                // to guarantee the caller sees exactly what was persisted
                // (millis-truncated, canonical form).
                self.get(&id).await
            },
            Err(sqlx::Error::Database(db_err))
                if db_err.kind() == sqlx::error::ErrorKind::UniqueViolation =>
            {
                // Distinguish a primary-key (id) collision from the
                // `(owner_id, name)` partial-unique-index collision: only the
                // former means this id already exists. A name collision on a
                // *new* id must NOT report `AlreadyExists { id }` (misleading).
                let msg = db_err.message();
                if msg.contains("credentials.id") {
                    Err(StoreError::AlreadyExists { id })
                } else {
                    Err(StoreError::Backend(
                        format!("credential unique-constraint violation (not id): {msg}").into(),
                    ))
                }
            },
            Err(e) => Err(StoreError::Backend(e.into())),
        }
    }

    /// `Overwrite`: UPSERT; version = existing + 1 (or 1 for a new row).
    ///
    /// Uses a two-step approach: attempt an INSERT first; on PK conflict,
    /// read the existing version and UPDATE with version+1. This avoids
    /// a conditional RETURNING clause that behaves differently across SQLite
    /// versions and simplifies the version-increment logic.
    ///
    /// The read-then-update is NOT atomic under concurrent writers — but
    /// `Overwrite` semantics do not guarantee atomicity (last-writer-wins is
    /// acceptable for this mode). Callers that need atomicity must use CAS.
    async fn put_overwrite(
        &self,
        credential: StoredCredential,
    ) -> Result<StoredCredential, StoreError> {
        let id = credential.id.clone();

        // Fetch the existing version if the row exists.
        let existing_version: Option<i64> =
            sqlx::query_as("SELECT version FROM credentials WHERE id = ?1")
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

        let now_ms = Utc::now().timestamp_millis();
        let expires_ms: Option<i64> = credential.expires_at.map(|dt| dt.timestamp_millis());
        let owner_id = owner_id_from_metadata(&credential.metadata).map(ToOwned::to_owned);
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = state_version_to_i64(credential.state_version);
        let reauth_i64: i64 = i64::from(credential.reauth_required);

        // UPSERT: INSERT or replace all mutable columns on PK conflict.
        // `created_at` is preserved on conflict (only updated when it's a new row).
        sqlx::query(
            "INSERT INTO credentials \
             (id, name, owner_id, credential_key, state_kind, state_version, \
              data, version, created_at, updated_at, expires_at, \
              reauth_required, metadata) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11, ?12) \
             ON CONFLICT(id) DO UPDATE SET \
               name            = excluded.name, \
               owner_id        = excluded.owner_id, \
               credential_key  = excluded.credential_key, \
               state_kind      = excluded.state_kind, \
               state_version   = excluded.state_version, \
               data            = excluded.data, \
               version         = ?8, \
               updated_at      = ?9, \
               expires_at      = excluded.expires_at, \
               reauth_required = excluded.reauth_required, \
               metadata        = excluded.metadata",
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(&credential.data)
        .bind(new_version)
        .bind(now_ms)
        .bind(expires_ms)
        .bind(reauth_i64)
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
        let now_ms = Utc::now().timestamp_millis();
        let expires_ms: Option<i64> = credential.expires_at.map(|dt| dt.timestamp_millis());
        let owner_id = owner_id_from_metadata(&credential.metadata).map(ToOwned::to_owned);
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = state_version_to_i64(credential.state_version);
        let reauth_i64: i64 = i64::from(credential.reauth_required);

        let result = sqlx::query(
            "UPDATE credentials SET \
               name            = ?2, \
               owner_id        = ?3, \
               credential_key  = ?4, \
               state_kind      = ?5, \
               state_version   = ?6, \
               data            = ?7, \
               version         = ?8, \
               updated_at      = ?9, \
               expires_at      = ?10, \
               reauth_required = ?11, \
               metadata        = ?12 \
             WHERE id = ?1 AND version = ?13",
        )
        .bind(&credential.id)
        .bind(&credential.name)
        .bind(&owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(&credential.data)
        .bind(new_version)
        .bind(now_ms)
        .bind(expires_ms)
        .bind(reauth_i64)
        .bind(&meta_json)
        .bind(expected_i64)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            // Either the row doesn't exist or the version didn't match.
            // Fetch to distinguish the two cases.
            let current: Option<(i64,)> =
                sqlx::query_as("SELECT version FROM credentials WHERE id = ?1")
                    .bind(&id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(|e| StoreError::Backend(e.into()))?;

            return match current {
                None => Err(StoreError::NotFound { id }),
                // A negative stored version is table corruption, not a normal
                // version mismatch; surface it as Backend rather than fabricating
                // an `actual` (which would mask the corruption as VersionConflict).
                Some((actual_i64,)) => match u64::try_from(actual_i64) {
                    Ok(actual) => Err(StoreError::VersionConflict {
                        id,
                        expected: expected_version,
                        actual,
                    }),
                    Err(_) => Err(StoreError::Backend(
                        format!("stored version {actual_i64} is negative — table corruption")
                            .into(),
                    )),
                },
            };
        }

        self.get(&id).await
    }
}
