//! SQLite-backed `CredentialPersistence` impl.
//!
//! Persists [`StoredCredential`] rows in the `credentials` table created by
//! migration `0030_credentials_store.sql`. The store is deliberately thin:
//!
//! - `data` is an opaque `BLOB` — the [`EncryptionLayer`] above us serialises
//!   the AES-256-GCM envelope; we never inspect or decrypt it.
//! - `owner_id` comes only from the mandatory selector, is included in every
//!   row predicate, and overwrites the compatibility metadata stamp on write.
//! - Timestamps are stored as `INTEGER` milliseconds-since-epoch (UTC), not
//!   RFC-3339 text, for the same reasons documented in the `RefreshClaimRepo`
//!   SQLite impl (`refresh_claim/sqlite.rs`): integer ordering is unambiguous
//!   for expiry predicates across chrono versions.
//! - CAS uses a conditional owner/id/version `UPDATE` and
//!   inspects `rows_affected()` to discriminate between `NotFound` and
//!   `VersionConflict`. The `u64 ↔ i64` boundary is guarded explicitly;
//!   there are no silent `as` casts.
//!
//! # Caller contract
//!
//! The caller (a composition root) is responsible for running migration 0030
//! before constructing a [`SqliteCredentialPersistence`].
//!
//! [`EncryptionLayer`]: crate::credential::layer::EncryptionLayer

// budget-justified: cohesive SQLite adapter for the credentials table +
// migration 0030 schema; one file mirrors the established refresh_claim/sqlite.rs
// adapter pattern (helpers + row mapping + trait impl + put dispatch).

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use nebula_storage_port::{
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
    CredentialWriteMode, OWNER_ID_METADATA_KEY, StoredCredential, StoredCredentialHead,
};
use serde_json::Value;
use sqlx::SqlitePool;

/// SQLite-backed [`CredentialPersistence`].
///
/// Wraps a `SqlitePool`. Cheap to clone (pool is `Arc`-backed).
/// Caller must have applied migration `0030_credentials_store.sql` to the pool.
#[derive(Clone, Debug)]
pub struct SqliteCredentialPersistence {
    pool: SqlitePool,
}

impl SqliteCredentialPersistence {
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
    /// Returns [`CredentialPersistenceError::Backend`] if the URL is malformed, the database
    /// cannot be opened/created, or the migration fails to apply.
    pub async fn connect(url: &str) -> Result<Self, CredentialPersistenceError> {
        use std::str::FromStr;

        let options = sqlx::sqlite::SqliteConnectOptions::from_str(url)
            .map_err(|e| backend(format!("invalid SQLite URL `{url}`: {e}")))?
            .create_if_missing(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(|e| backend(format!("open SQLite `{url}`: {e}")))?;

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
        .map_err(|e| backend(format!("probe credentials table: {e}")))?;
        if provisioned.is_none() {
            sqlx::query(include_str!(
                "../../migrations/sqlite/0030_credentials_store.sql"
            ))
            .execute(&pool)
            .await
            .map_err(|e| backend(format!("apply migration 0030: {e}")))?;
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
    /// Creates a unique shared-cache in-memory SQLite store for testing.
    ///
    /// Available whenever the `sqlite` feature is enabled. Intended for unit and
    /// integration tests; production composition roots use [`Self::connect`]
    /// with a real URL instead.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialPersistenceError::Backend`] if the pool cannot be opened or migration
    /// 0030 fails to apply.
    pub async fn connect_memory() -> Result<Self, CredentialPersistenceError> {
        use std::str::FromStr;

        let name = uuid::Uuid::new_v4();
        let url = format!("sqlite:file:nebula-cred-mem-{name}?mode=memory&cache=shared");
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
            .map_err(|e| backend(format!("invalid SQLite URL `{url}`: {e}")))?
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
            .map_err(|e| backend(format!("open in-memory SQLite `{url}`: {e}")))?;

        sqlx::query(include_str!(
            "../../migrations/sqlite/0030_credentials_store.sql"
        ))
        .execute(&pool)
        .await
        .map_err(|e| backend(format!("apply migration 0030: {e}")))?;

        Ok(Self { pool })
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn backend(message: impl Into<String>) -> CredentialPersistenceError {
    CredentialPersistenceError::Backend(Box::new(std::io::Error::other(message.into())))
}

/// Convert a millisecond-since-epoch `INTEGER` column back to `DateTime<Utc>`.
///
/// Mirrors the identical helper in `refresh_claim/sqlite.rs`. An out-of-range
/// value indicates table corruption; surfaced as `CredentialPersistenceError::Backend`.
fn millis_to_utc(ms: i64, col: &'static str) -> Result<DateTime<Utc>, CredentialPersistenceError> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or_else(|| backend(format!("{col} millis out of range: {ms}")))
}

/// Guard a `u64` version counter against i64 overflow before writing to SQLite.
fn version_to_i64(v: u64) -> Result<i64, CredentialPersistenceError> {
    i64::try_from(v).map_err(|_| backend(format!("version {v} overflows i64 (SQLite INTEGER)")))
}

/// Guard a `u64` state_version (u32 in the DTO but stored as INTEGER) cast.
fn state_version_to_i64(v: u32) -> i64 {
    i64::from(v)
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
    created_at: i64,
    updated_at: i64,
    expires_at: Option<i64>,
    reauth_required: i64,
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

// ── CredentialPersistence impl ──────────────────────────────────────────────────────

#[async_trait]
impl CredentialPersistence for SqliteCredentialPersistence {
    #[tracing::instrument(skip(self, selector), fields(credential.id = selector.credential_id()))]
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata \
             FROM credentials WHERE id = ?1 AND owner_id = ?2",
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
             FROM credentials WHERE id = ?1 AND owner_id = ?2",
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
        let result = sqlx::query("DELETE FROM credentials WHERE id = ?1 AND owner_id = ?2")
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
                sqlx::query_as("SELECT id FROM credentials WHERE owner_id = ?1 AND state_kind = ?2")
                    .bind(owner.as_str())
                    .bind(kind)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| CredentialPersistenceError::Backend(e.into()))?
            },
            None => sqlx::query_as("SELECT id FROM credentials WHERE owner_id = ?1")
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
                 FROM credentials WHERE owner_id = ?1 AND state_kind = ?2 ORDER BY id",
            )
            .bind(owner.as_str())
            .bind(kind)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CredentialPersistenceError::Backend(e.into()))?,
            None => sqlx::query_as(
                "SELECT id, name, credential_key, state_kind, state_version, version, \
                 created_at, updated_at, expires_at, reauth_required, metadata \
                 FROM credentials WHERE owner_id = ?1 ORDER BY id",
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
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM credentials WHERE id = ?1 AND owner_id = ?2 LIMIT 1")
                .bind(selector.credential_id())
                .bind(selector.owner().as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;
        Ok(row.is_some())
    }
}

// ── put dispatch helpers ──────────────────────────────────────────────────────

impl SqliteCredentialPersistence {
    /// `CreateOnly`: INSERT; fail with `AlreadyExists` on PRIMARY KEY conflict.
    async fn put_create_only(
        &self,
        selector: &CredentialSelector,
        credential: StoredCredential,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let id = credential.id.clone();
        let now = Utc::now();
        let created_ms = now.timestamp_millis();
        let expires_ms: Option<i64> = credential.expires_at.map(|dt| dt.timestamp_millis());
        let owner_id = selector.owner().as_str();
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
        .bind(owner_id)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(credential.data.as_ref())
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
                self.get(selector).await
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
                    if self.exists(selector).await? {
                        Err(CredentialPersistenceError::AlreadyExists { credential_id: id })
                    } else {
                        Err(CredentialPersistenceError::NotFound { credential_id: id })
                    }
                } else {
                    Err(backend(format!(
                        "credential unique-constraint violation (not id): {msg}"
                    )))
                }
            },
            Err(e) => Err(CredentialPersistenceError::Backend(e.into())),
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
        selector: &CredentialSelector,
        credential: StoredCredential,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let id = credential.id.clone();

        // Fetch the existing version if the row exists.
        let existing_version: Option<i64> =
            sqlx::query_as("SELECT version FROM credentials WHERE id = ?1 AND owner_id = ?2")
                .bind(&id)
                .bind(selector.owner().as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CredentialPersistenceError::Backend(e.into()))?
                .map(|(v,): (i64,)| v);

        let Some(existing_version) = existing_version else {
            return self.put_create_only(selector, credential).await;
        };
        let new_version: i64 = {
            let v_u64 = u64::try_from(existing_version).map_err(|_| {
                backend(format!(
                    "stored version {existing_version} is negative — table corruption"
                ))
            })?;
            version_to_i64(v_u64.saturating_add(1))?
        };

        let now_ms = Utc::now().timestamp_millis();
        let expires_ms: Option<i64> = credential.expires_at.map(|dt| dt.timestamp_millis());
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = state_version_to_i64(credential.state_version);
        let reauth_i64: i64 = i64::from(credential.reauth_required);

        let result = sqlx::query(
            "UPDATE credentials SET \
               name            = ?3, \
               credential_key  = ?4, \
               state_kind      = ?5, \
               state_version   = ?6, \
               data            = ?7, \
               version         = ?8, \
               updated_at      = ?9, \
               expires_at      = ?10, \
               reauth_required = ?11, \
               metadata        = ?12 \
             WHERE id = ?1 AND owner_id = ?2",
        )
        .bind(&credential.id)
        .bind(selector.owner().as_str())
        .bind(&credential.name)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(credential.data.as_ref())
        .bind(new_version)
        .bind(now_ms)
        .bind(expires_ms)
        .bind(reauth_i64)
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
        let now_ms = Utc::now().timestamp_millis();
        let expires_ms: Option<i64> = credential.expires_at.map(|dt| dt.timestamp_millis());
        let meta_json = meta_to_json(&credential.metadata)?;
        let state_version_i64 = state_version_to_i64(credential.state_version);
        let reauth_i64: i64 = i64::from(credential.reauth_required);

        let result = sqlx::query(
            "UPDATE credentials SET \
               name            = ?3, \
               credential_key  = ?4, \
               state_kind      = ?5, \
               state_version   = ?6, \
               data            = ?7, \
               version         = ?8, \
               updated_at      = ?9, \
               expires_at      = ?10, \
               reauth_required = ?11, \
               metadata        = ?12 \
             WHERE id = ?1 AND owner_id = ?2 AND version = ?13",
        )
        .bind(&credential.id)
        .bind(selector.owner().as_str())
        .bind(&credential.name)
        .bind(&credential.credential_key)
        .bind(&credential.state_kind)
        .bind(state_version_i64)
        .bind(credential.data.as_ref())
        .bind(new_version)
        .bind(now_ms)
        .bind(expires_ms)
        .bind(reauth_i64)
        .bind(&meta_json)
        .bind(expected_i64)
        .execute(&self.pool)
        .await
        .map_err(|e| CredentialPersistenceError::Backend(e.into()))?;

        if result.rows_affected() == 0 {
            // Either the row doesn't exist or the version didn't match.
            // Fetch to distinguish the two cases.
            let current: Option<(i64,)> =
                sqlx::query_as("SELECT version FROM credentials WHERE id = ?1 AND owner_id = ?2")
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
