//! Postgres-backed `CredentialPersistence` impl.
//!
//! Persists structural [`StoredCredential`] rows in the `credentials` table at
//! schema floor `0040_credential_refresh_retry_gate.sql`. The store is
//! deliberately linear:
//!
//! - `data` is opaque `BYTEA`; the encryption layer above this adapter owns its
//!   representation.
//! - `owner_id` comes only from the mandatory selector and is included in every
//!   row predicate.
//! - Every mutation uses one ordinary transaction, locks an existing physical
//!   row before classification, executes one guarded DML statement with
//!   `RETURNING`, and acknowledges only after `COMMIT`.
//! - A failure before commit dispatch is [`CredentialPersistenceError::Unavailable`].
//!   Once commit is dispatched without acknowledgement it is
//!   [`CredentialPersistenceError::OutcomeUnknown`], and the adapter never
//!   retries.
//!
//! [`Self::connect`](PgCredentialPersistence::connect) and
//! [`Self::connect_with`](PgCredentialPersistence::connect_with) are the only
//! constructors; no unchecked raw-pool constructor can bypass readiness.

// budget-justified: one cohesive PostgreSQL adapter owns readiness, physical-row
// decoding, mutation classification, and the commit-acknowledgement boundary.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialMaterialEpoch,
    CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialReplacement,
    CredentialSelector, CredentialTombstone, CredentialVersion, RefreshRetrySnapshot, SecretBytes,
    StoredCredential, StoredCredentialHead, StoredLiveCredential, StoredTombstonedCredential,
};
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::fmt;
#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::{
    CredentialStoreStartupError,
    refresh_claim::PgRefreshClaimRepo,
    retry_gate,
    schema::{postgres as schema, unlocked_postgres_migrator},
};

/// PostgreSQL-backed [`CredentialPersistence`].
///
/// The internal pool is cheap to clone, but construction always passes through
/// the schema admission and migration gate.
#[derive(Clone)]
pub struct PgCredentialPersistence {
    pool: PgPool,
    #[cfg(test)]
    lose_next_commit_acknowledgement: Arc<AtomicBool>,
}

impl fmt::Debug for PgCredentialPersistence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PgCredentialPersistence")
    }
}

impl PgCredentialPersistence {
    fn from_admitted_pool(pool: PgPool) -> Self {
        Self {
            pool,
            #[cfg(test)]
            lose_next_commit_acknowledgement: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Connect, admit the canonical schema, and apply all pending migrations
    /// under the bounded PostgreSQL readiness lock.
    ///
    /// # Errors
    ///
    /// Returns a closed, secret-free startup error. URLs and driver messages
    /// are never retained by the error.
    pub async fn connect(url: &str) -> Result<Self, CredentialStoreStartupError> {
        use std::str::FromStr;

        let options = sqlx::postgres::PgConnectOptions::from_str(url)
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        Self::connect_with(options).await
    }

    /// Connect with explicit SQLx options while preserving the same mandatory
    /// readiness and migration gate as [`Self::connect`].
    ///
    /// # Errors
    ///
    /// Returns a closed, secret-free startup error when connection, admission,
    /// locking, migration, or postflight fails.
    pub async fn connect_with(
        options: sqlx::postgres::PgConnectOptions,
    ) -> Result<Self, CredentialStoreStartupError> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .min_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        let mut connection = pool
            .acquire()
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        let lock_key = postgres_lock_key(&mut connection).await?;
        acquire_postgres_lock(&mut connection, lock_key).await?;

        let readiness = async {
            read_only_admission(&mut connection).await?;
            unlocked_postgres_migrator()
                .run(&mut *connection)
                .await
                .map_err(|_| CredentialStoreStartupError::Unavailable)?;
            read_only_admission(&mut connection).await
        }
        .await;

        let unlocked = sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
            .bind(lock_key)
            .fetch_one(&mut *connection)
            .await
            .unwrap_or(false);
        if !unlocked {
            return Err(CredentialStoreStartupError::Unavailable);
        }
        readiness?;
        drop(connection);

        Ok(Self::from_admitted_pool(pool))
    }

    /// Create the refresh-claim adapter on this store's admitted private pool.
    ///
    /// This is the supported composition seam for pairing owner-bound
    /// credential persistence with durable cross-replica refresh
    /// coordination. It clones the pool handle without exposing raw SQL
    /// authority, so both adapters share one schema lifecycle and one
    /// PostgreSQL database.
    #[must_use]
    pub fn refresh_claim_repo(&self) -> PgRefreshClaimRepo {
        PgRefreshClaimRepo::new(self.pool.clone())
    }

    async fn begin_mutation(
        &self,
    ) -> Result<Transaction<'_, Postgres>, CredentialPersistenceError> {
        self.pool
            .begin()
            .await
            .map_err(|_| CredentialPersistenceError::Unavailable)
    }

    async fn commit_acknowledged(
        &self,
        transaction: Transaction<'_, Postgres>,
        commit: CredentialCommit,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        match self.dispatch_commit(transaction).await {
            Ok(()) => Ok(commit),
            Err(CommitDispatchError::DefiniteFailure) => {
                Err(CredentialPersistenceError::Unavailable)
            },
            Err(CommitDispatchError::AcknowledgementLost) => {
                Err(CredentialPersistenceError::OutcomeUnknown)
            },
        }
    }

    /// Dispatch `COMMIT` across the private driver boundary.
    ///
    /// The caller receives only an authoritative acknowledgement, a definite
    /// failure, or a lost acknowledgement. The test transport can consume one
    /// acknowledgement after the physical commit without revealing the
    /// underlying success to the adapter classifier.
    async fn dispatch_commit(
        &self,
        transaction: Transaction<'_, Postgres>,
    ) -> Result<(), CommitDispatchError> {
        #[cfg(test)]
        let lose_acknowledgement = self
            .lose_next_commit_acknowledgement
            .swap(false, Ordering::AcqRel);

        match transaction.commit().await {
            Ok(()) => {
                #[cfg(test)]
                if lose_acknowledgement {
                    return Err(CommitDispatchError::AcknowledgementLost);
                }
                Ok(())
            },
            Err(sqlx::Error::Database(database))
                if is_unknown_commit_sqlstate(database.code().as_deref()) =>
            {
                Err(CommitDispatchError::AcknowledgementLost)
            },
            // PostgreSQL returned an authoritative transaction error, so this
            // transaction is known not to have committed.
            Err(sqlx::Error::Database(_)) => Err(CommitDispatchError::DefiniteFailure),
            // A transport/protocol failure after COMMIT dispatch has no
            // authoritative acknowledgement. The adapter deliberately does not retry.
            Err(_) => Err(CommitDispatchError::AcknowledgementLost),
        }
    }

    #[cfg(test)]
    fn lose_next_commit_acknowledgement(&self) {
        self.lose_next_commit_acknowledgement
            .store(true, Ordering::Release);
    }

    async fn classify_create_unique_collision(
        &self,
        selector: &CredentialSelector,
        name_collision: bool,
    ) -> CredentialPersistenceError {
        let credential_id = selector.credential_id().to_string();
        let existing: Result<Option<ExistingCredentialRow>, sqlx::Error> = sqlx::query_as(
            "SELECT owner_id = $2 AS is_same_owner, record_state
             FROM credentials
             WHERE id = $1",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await;

        match existing {
            Ok(Some(row)) => classify_existing_id(row),
            Ok(None) if name_collision => CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Name,
            },
            Ok(None) => CredentialPersistenceError::Unavailable,
            Err(error) => read_error(error),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitDispatchError {
    DefiniteFailure,
    AcknowledgementLost,
}

fn is_unknown_commit_sqlstate(code: Option<&str>) -> bool {
    matches!(code, Some("40003"))
        || code.is_some_and(|code| {
            // SQLSTATE class 08 is a connection exception. Once COMMIT has
            // been dispatched, none of its members authoritatively proves
            // whether the server committed before the connection failed.
            code.starts_with("08")
        })
}

async fn postgres_lock_key(
    connection: &mut sqlx::PgConnection,
) -> Result<i64, CredentialStoreStartupError> {
    sqlx::query_scalar(
        "SELECT hashtextextended(
             'nebula:credential-schema:' || current_database() || ':' || current_schema(),
             0
         )",
    )
    .fetch_one(connection)
    .await
    .map_err(|_| CredentialStoreStartupError::Unavailable)
}

async fn acquire_postgres_lock(
    connection: &mut sqlx::PgConnection,
    lock_key: i64,
) -> Result<(), CredentialStoreStartupError> {
    use std::time::Duration;

    for _ in 0..200 {
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        if acquired {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err(CredentialStoreStartupError::Unavailable)
}

async fn read_only_admission(
    connection: &mut sqlx::PgConnection,
) -> Result<(), CredentialStoreStartupError> {
    sqlx::query("BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    let admission = schema::admit(&mut *connection).await;
    let finish = if admission.is_ok() {
        "COMMIT"
    } else {
        "ROLLBACK"
    };
    sqlx::query(finish)
        .execute(&mut *connection)
        .await
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    admission.map(|_| ())
}

fn encode_metadata(metadata: &Map<String, Value>) -> Result<String, CredentialPersistenceError> {
    serde_json::to_string(metadata).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn decode_metadata(metadata: &str) -> Result<Map<String, Value>, CredentialPersistenceError> {
    serde_json::from_str(metadata).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn read_error(error: sqlx::Error) -> CredentialPersistenceError {
    match error {
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. }
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::RowNotFound => CredentialPersistenceError::CorruptRecord,
        _ => CredentialPersistenceError::Unavailable,
    }
}

fn validate_name_projection(
    name: Option<&str>,
    metadata: &Map<String, Value>,
) -> Result<(), CredentialPersistenceError> {
    let projected_name = match metadata.get("display") {
        None => None,
        Some(Value::Object(display)) => {
            for (key, value) in display {
                match key.as_str() {
                    "display_name" | "description" if value.is_null() || value.is_string() => {},
                    "tags" => {
                        let Value::Object(tags) = value else {
                            return Err(CredentialPersistenceError::CorruptRecord);
                        };
                        if tags.values().any(|tag| !tag.is_string()) {
                            return Err(CredentialPersistenceError::CorruptRecord);
                        }
                    },
                    "display_name" | "description" => {
                        return Err(CredentialPersistenceError::CorruptRecord);
                    },
                    _ => {},
                }
            }
            display.get("display_name").and_then(Value::as_str)
        },
        Some(_) => return Err(CredentialPersistenceError::CorruptRecord),
    };

    if name != projected_name {
        return Err(CredentialPersistenceError::CorruptRecord);
    }
    Ok(())
}

fn parse_credential_id(value: &str) -> Result<CredentialId, CredentialPersistenceError> {
    value
        .parse()
        .map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn parse_version(value: i64) -> Result<CredentialVersion, CredentialPersistenceError> {
    CredentialVersion::try_from(value).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn parse_material_epoch(value: i64) -> Result<CredentialMaterialEpoch, CredentialPersistenceError> {
    CredentialMaterialEpoch::try_from(value).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn parse_state_version(value: i64) -> Result<u32, CredentialPersistenceError> {
    u32::try_from(value).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn classify_existing_id(row: ExistingCredentialRow) -> CredentialPersistenceError {
    if !row.is_same_owner {
        return CredentialPersistenceError::NotFound;
    }
    match row.record_state.as_str() {
        "live" | "tombstoned" => CredentialPersistenceError::AlreadyExists {
            key: CredentialAlreadyExistsKey::Id,
        },
        _ => CredentialPersistenceError::CorruptRecord,
    }
}

fn classify_locked_live(
    row: &LockedCredentialRow,
    expected: CredentialVersion,
) -> Result<CredentialVersion, CredentialPersistenceError> {
    if row.record_state == "tombstoned" {
        return Err(CredentialPersistenceError::NotFound);
    }
    if row.record_state != "live" {
        return Err(CredentialPersistenceError::CorruptRecord);
    }
    let actual = parse_version(row.version)?;
    if actual != expected {
        return Err(CredentialPersistenceError::VersionConflict { expected, actual });
    }
    Ok(actual)
}

fn is_id_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if database.kind() == sqlx::error::ErrorKind::UniqueViolation
                && database.constraint() == Some("credentials_pkey")
    )
}

fn is_name_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if database.kind() == sqlx::error::ErrorKind::UniqueViolation
                && database.constraint() == Some("idx_credentials_owner_name")
    )
}

async fn rollback_as<T>(
    transaction: Transaction<'_, Postgres>,
    error: CredentialPersistenceError,
) -> Result<T, CredentialPersistenceError> {
    let _ = transaction.rollback().await;
    Err(error)
}

async fn lock_credential_for_create(
    transaction: &mut Transaction<'_, Postgres>,
    credential_id: &str,
    owner: &CredentialOwner,
) -> Result<Option<ExistingCredentialRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT owner_id = $2 AS is_same_owner, record_state
         FROM credentials
         WHERE id = $1
         FOR UPDATE",
    )
    .bind(credential_id)
    .bind(owner.as_str())
    .fetch_optional(&mut **transaction)
    .await
}

async fn lock_owner_credential(
    transaction: &mut Transaction<'_, Postgres>,
    credential_id: &str,
    owner: &CredentialOwner,
) -> Result<Option<LockedCredentialRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT record_state, version, material_epoch
         FROM credentials
         WHERE id = $1 AND owner_id = $2
         FOR UPDATE",
    )
    .bind(credential_id)
    .bind(owner.as_str())
    .fetch_optional(&mut **transaction)
    .await
}

#[derive(sqlx::FromRow)]
struct ExistingCredentialRow {
    is_same_owner: bool,
    record_state: String,
}

#[derive(sqlx::FromRow)]
struct LockedCredentialRow {
    record_state: String,
    version: i64,
    material_epoch: i64,
}

#[derive(sqlx::FromRow)]
struct CredentialRow {
    id: String,
    name: Option<String>,
    credential_key: String,
    data: Vec<u8>,
    state_kind: String,
    state_version: i64,
    version: i64,
    material_epoch: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    reauth_required: bool,
    metadata: String,
    record_state: String,
    tombstoned_at: Option<DateTime<Utc>>,
    refresh_retry_mode: Option<String>,
    refresh_retry_not_before: Option<DateTime<Utc>>,
    refresh_retry_phase: Option<String>,
    refresh_retry_kind: Option<String>,
    refresh_retry_diagnostic_code: Option<String>,
}

impl CredentialRow {
    fn into_stored(self) -> Result<StoredCredential, CredentialPersistenceError> {
        let credential_id = parse_credential_id(&self.id)?;
        let version = parse_version(self.version)?;
        let material_epoch = parse_material_epoch(self.material_epoch)?;
        let state_version = parse_state_version(self.state_version)?;
        let refresh_retry_gate = retry_gate::decode_gate(
            self.refresh_retry_mode,
            self.refresh_retry_not_before,
            self.refresh_retry_phase,
            self.refresh_retry_kind,
            self.refresh_retry_diagnostic_code,
        )?;

        match self.record_state.as_str() {
            "live" => {
                if self.tombstoned_at.is_some() {
                    return Err(CredentialPersistenceError::CorruptRecord);
                }
                let metadata = decode_metadata(&self.metadata)?;
                validate_name_projection(self.name.as_deref(), &metadata)?;
                StoredLiveCredential::new(
                    credential_id,
                    self.name,
                    self.credential_key,
                    SecretBytes::new(self.data),
                    self.state_kind,
                    state_version,
                    version,
                    material_epoch,
                    self.created_at,
                    self.updated_at,
                    self.expires_at,
                    self.reauth_required,
                    metadata,
                    refresh_retry_gate,
                )
                .map(StoredCredential::Live)
            },
            "tombstoned" => {
                let Some(tombstoned_at) = self.tombstoned_at else {
                    return Err(CredentialPersistenceError::CorruptRecord);
                };
                if self.name.is_some()
                    || !self.data.is_empty()
                    || self.expires_at.is_some()
                    || self.reauth_required
                    || self.metadata != "{}"
                    || refresh_retry_gate.is_some()
                {
                    return Err(CredentialPersistenceError::CorruptRecord);
                }
                Ok(StoredCredential::Tombstoned(
                    StoredTombstonedCredential::new(
                        credential_id,
                        self.credential_key,
                        self.state_kind,
                        state_version,
                        version,
                        self.created_at,
                        self.updated_at,
                        tombstoned_at,
                    ),
                ))
            },
            _ => Err(CredentialPersistenceError::CorruptRecord),
        }
    }
}

#[derive(sqlx::FromRow)]
struct CredentialHeadRow {
    id: String,
    name: Option<String>,
    credential_key: String,
    state_kind: String,
    state_version: i64,
    version: i64,
    material_epoch: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    reauth_required: bool,
    metadata: String,
    record_state: String,
    tombstoned_at: Option<DateTime<Utc>>,
}

impl CredentialHeadRow {
    fn into_stored_head(self) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        if self.record_state != "live" || self.tombstoned_at.is_some() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        let metadata = decode_metadata(&self.metadata)?;
        validate_name_projection(self.name.as_deref(), &metadata)?;
        StoredCredentialHead::new(
            parse_credential_id(&self.id)?,
            self.name,
            self.credential_key,
            self.state_kind,
            parse_state_version(self.state_version)?,
            parse_version(self.version)?,
            parse_material_epoch(self.material_epoch)?,
            self.created_at,
            self.updated_at,
            self.expires_at,
            self.reauth_required,
            metadata,
        )
    }
}

#[derive(sqlx::FromRow)]
struct CredentialCommitRow {
    id: String,
    version: i64,
    record_state: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct RefreshRetrySnapshotRow {
    version: i64,
    material_epoch: i64,
    reauth_required: bool,
    record_state: String,
    refresh_retry_mode: Option<String>,
    refresh_retry_not_before: Option<DateTime<Utc>>,
    refresh_retry_phase: Option<String>,
    refresh_retry_kind: Option<String>,
    refresh_retry_diagnostic_code: Option<String>,
    backend_now: DateTime<Utc>,
}

impl RefreshRetrySnapshotRow {
    fn into_snapshot(self) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        if self.record_state != "live" {
            return if self.record_state == "tombstoned" {
                Err(CredentialPersistenceError::NotFound)
            } else {
                Err(CredentialPersistenceError::CorruptRecord)
            };
        }
        let version = CredentialVersion::try_from(self.version)
            .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
        if !version.is_live() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        let material_epoch = parse_material_epoch(self.material_epoch)?;
        let gate = retry_gate::decode_gate(
            self.refresh_retry_mode,
            self.refresh_retry_not_before,
            self.refresh_retry_phase,
            self.refresh_retry_kind,
            self.refresh_retry_diagnostic_code,
        )?;
        let admission = retry_gate::evaluate_gate(gate.as_ref(), self.backend_now)?;
        Ok(RefreshRetrySnapshot::new(
            version,
            material_epoch,
            self.reauth_required,
            admission,
        ))
    }
}

impl CredentialCommitRow {
    fn into_commit(self) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = parse_credential_id(&self.id)?;
        let version = parse_version(self.version)?;
        match self.record_state.as_str() {
            "live" => {
                if self.tombstoned_at.is_some() {
                    return Err(CredentialPersistenceError::CorruptRecord);
                }
                CredentialCommit::live(credential_id, version, self.created_at, self.updated_at)
            },
            "tombstoned" => {
                let Some(tombstoned_at) = self.tombstoned_at else {
                    return Err(CredentialPersistenceError::CorruptRecord);
                };
                Ok(CredentialCommit::tombstoned(
                    credential_id,
                    version,
                    self.created_at,
                    self.updated_at,
                    tombstoned_at,
                ))
            },
            _ => Err(CredentialPersistenceError::CorruptRecord),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl super::CredentialPersistenceConformance for PgCredentialPersistence {
    async fn force_live_version_for_conformance(
        &self,
        selector: &CredentialSelector,
        version: CredentialVersion,
    ) -> Result<(), CredentialPersistenceError> {
        if !version.is_live() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        let updated = sqlx::query(
            "UPDATE credentials SET version = $1
             WHERE id = $2 AND owner_id = $3 AND record_state = 'live'",
        )
        .bind(version.get())
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .execute(&self.pool)
        .await
        .map_err(read_error)?;
        if updated.rows_affected() != 1 {
            return Err(CredentialPersistenceError::NotFound);
        }
        Ok(())
    }

    async fn force_live_material_epoch_for_conformance(
        &self,
        selector: &CredentialSelector,
        material_epoch: CredentialMaterialEpoch,
    ) -> Result<(), CredentialPersistenceError> {
        let updated = sqlx::query(
            "UPDATE credentials SET material_epoch = $1
             WHERE id = $2 AND owner_id = $3 AND record_state = 'live'",
        )
        .bind(material_epoch.get())
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .execute(&self.pool)
        .await
        .map_err(read_error)?;
        if updated.rows_affected() != 1 {
            return Err(CredentialPersistenceError::NotFound);
        }
        Ok(())
    }

    async fn corrupt_live_projection_for_conformance(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), CredentialPersistenceError> {
        let updated = sqlx::query(
            "UPDATE credentials
             SET name = NULL, metadata = '{\"display\":\"not-an-object\"}'
             WHERE id = $1 AND owner_id = $2 AND record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .execute(&self.pool)
        .await
        .map_err(read_error)?;
        if updated.rows_affected() != 1 {
            return Err(CredentialPersistenceError::NotFound);
        }
        Ok(())
    }
}

#[async_trait]
impl CredentialPersistence for PgCredentialPersistence {
    #[tracing::instrument(skip_all)]
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version,
                    version, material_epoch, created_at, updated_at, expires_at,
                    reauth_required, metadata, record_state, tombstoned_at,
                    refresh_retry_mode, refresh_retry_not_before,
                    refresh_retry_phase, refresh_retry_kind,
                    refresh_retry_diagnostic_code
             FROM credentials
             WHERE id = $1 AND owner_id = $2",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;

        row.ok_or(CredentialPersistenceError::NotFound)?
            .into_stored()
    }

    #[tracing::instrument(skip_all)]
    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        let row: Option<RefreshRetrySnapshotRow> = sqlx::query_as(
            "SELECT version, material_epoch, reauth_required, record_state, refresh_retry_mode,
                    refresh_retry_not_before, refresh_retry_phase,
                    refresh_retry_kind, refresh_retry_diagnostic_code,
                    -- Sample the wall clock in the statement that observes
                    -- version and reauthentication state.
                    clock_timestamp() AS backend_now
             FROM credentials
             WHERE id = $1 AND owner_id = $2",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;

        row.ok_or(CredentialPersistenceError::NotFound)?
            .into_snapshot()
    }

    #[tracing::instrument(skip_all)]
    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let row: Option<CredentialHeadRow> = sqlx::query_as(
            "SELECT id, name, credential_key, state_kind, state_version,
                    version, material_epoch, created_at, updated_at, expires_at,
                    reauth_required, metadata, record_state, tombstoned_at
             FROM credentials
             WHERE id = $1 AND owner_id = $2 AND record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;

        row.ok_or(CredentialPersistenceError::NotFound)?
            .into_stored_head()
    }

    #[tracing::instrument(skip_all)]
    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let name = create.name().map(str::to_owned);
        validate_name_projection(name.as_deref(), create.metadata())?;
        let metadata = encode_metadata(create.metadata())?;
        let mut transaction = self.begin_mutation().await?;

        let existing =
            match lock_credential_for_create(&mut transaction, &credential_id, selector.owner())
                .await
            {
                Ok(existing) => existing,
                Err(error) => {
                    return rollback_as(transaction, read_error(error)).await;
                },
            };
        if let Some(existing) = existing {
            let error = classify_existing_id(existing);
            return rollback_as(transaction, error).await;
        }

        let inserted: Result<CredentialCommitRow, sqlx::Error> = sqlx::query_as(
            "INSERT INTO credentials (
                 id, name, owner_id, credential_key, state_kind, state_version,
                 data, version, material_epoch, created_at, updated_at, expires_at,
                 reauth_required, metadata, record_state, tombstoned_at
             ) VALUES (
                 $1, $2, $3, $4, $5, $6,
                 $7, 1, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, $8,
                 $9, $10, 'live', NULL
             )
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(name.as_deref())
        .bind(selector.owner().as_str())
        .bind(create.credential_key())
        .bind(create.state_kind())
        .bind(i64::from(create.state_version()))
        .bind(create.data().as_ref())
        .bind(create.expires_at())
        .bind(create.reauth_required())
        .bind(&metadata)
        .fetch_one(&mut *transaction)
        .await;

        let inserted = match inserted {
            Ok(inserted) => inserted,
            Err(error) => {
                let id_collision = is_id_unique_violation(&error);
                let name_collision = is_name_unique_violation(&error);
                let failure = read_error(error);
                let _ = transaction.rollback().await;
                if id_collision || name_collision {
                    // Re-read after rollback and classify the global id before
                    // the owner-local name. A dual collision can be reported
                    // by either PostgreSQL constraint; API precedence must not
                    // depend on backend constraint-report ordering.
                    return Err(self
                        .classify_create_unique_collision(selector, name_collision)
                        .await);
                }
                return Err(failure);
            },
        };
        let commit = match inserted.into_commit() {
            Ok(commit) => commit,
            Err(error) => return rollback_as(transaction, error).await,
        };
        self.commit_acknowledged(transaction, commit).await
    }

    #[tracing::instrument(skip_all)]
    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let name = replacement.name().map(str::to_owned);
        validate_name_projection(name.as_deref(), replacement.metadata())?;
        let metadata = encode_metadata(replacement.metadata())?;
        let expected = replacement.expected_version();
        let mut transaction = self.begin_mutation().await?;

        let Some(locked) = (match lock_owner_credential(
            &mut transaction,
            &credential_id,
            selector.owner(),
        )
        .await
        {
            Ok(locked) => locked,
            Err(error) => {
                return rollback_as(transaction, read_error(error)).await;
            },
        }) else {
            return rollback_as(transaction, CredentialPersistenceError::NotFound).await;
        };
        let actual = match classify_locked_live(&locked, expected) {
            Ok(actual) => actual,
            Err(error) => return rollback_as(transaction, error).await,
        };
        let next_version = match actual.next_live() {
            Ok(next_version) => next_version,
            Err(error) => return rollback_as(transaction, error).await,
        };
        let actual_material_epoch = match parse_material_epoch(locked.material_epoch) {
            Ok(epoch) => epoch,
            Err(error) => return rollback_as(transaction, error).await,
        };
        let next_material_epoch = if replacement.material_transition().advances_epoch() {
            match actual_material_epoch.next() {
                Ok(epoch) => epoch,
                Err(error) => return rollback_as(transaction, error).await,
            }
        } else {
            actual_material_epoch
        };
        let retry_transition =
            match retry_gate::encode_material_transition(replacement.material_transition()) {
                Ok(transition) => transition,
                Err(error) => return rollback_as(transaction, error).await,
            };

        let updated: Result<Option<CredentialCommitRow>, sqlx::Error> = sqlx::query_as(
            "UPDATE credentials
             SET name = $3,
                 data = $4,
                 state_kind = $5,
                 state_version = $6,
                 version = $7,
                 material_epoch = $8,
                 updated_at = CURRENT_TIMESTAMP,
                 expires_at = $9,
                 reauth_required = $10,
                 metadata = $11,
                 refresh_retry_mode = CASE $12::SMALLINT
                     WHEN 0 THEN refresh_retry_mode
                     WHEN 1 THEN NULL
                     WHEN 2 THEN 'never'
                     WHEN 3 THEN 'not_before'
                 END,
                 refresh_retry_not_before = CASE $12::SMALLINT
                     WHEN 0 THEN refresh_retry_not_before
                     -- The row lock may have waited. CURRENT_TIMESTAMP would
                     -- backdate the requested delay to transaction start.
                     WHEN 3 THEN clock_timestamp() + ($13::BIGINT * INTERVAL '1 second')
                     ELSE NULL
                 END,
                 refresh_retry_phase = CASE $12::SMALLINT
                     WHEN 0 THEN refresh_retry_phase
                     WHEN 1 THEN NULL
                     ELSE $14
                 END,
                 refresh_retry_kind = CASE $12::SMALLINT
                     WHEN 0 THEN refresh_retry_kind
                     WHEN 1 THEN NULL
                     ELSE $15
                 END,
                 refresh_retry_diagnostic_code = CASE $12::SMALLINT
                     WHEN 0 THEN refresh_retry_diagnostic_code
                     WHEN 1 THEN NULL
                     ELSE $16
                 END
             WHERE id = $1
               AND owner_id = $2
               AND record_state = 'live'
               AND version = $17
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .bind(name.as_deref())
        .bind(replacement.data().as_ref())
        .bind(replacement.state_kind())
        .bind(i64::from(replacement.state_version()))
        .bind(next_version.get())
        .bind(next_material_epoch.get())
        .bind(replacement.expires_at())
        .bind(replacement.reauth_required())
        .bind(&metadata)
        .bind(retry_transition.code)
        .bind(retry_transition.delay_seconds)
        .bind(retry_transition.phase)
        .bind(retry_transition.kind)
        .bind(retry_transition.diagnostic_code)
        .bind(expected.get())
        .fetch_optional(&mut *transaction)
        .await;

        let updated = match updated {
            Ok(Some(updated)) => updated,
            Ok(None) => {
                return rollback_as(transaction, CredentialPersistenceError::Unavailable).await;
            },
            Err(error) if is_name_unique_violation(&error) => {
                return rollback_as(
                    transaction,
                    CredentialPersistenceError::AlreadyExists {
                        key: CredentialAlreadyExistsKey::Name,
                    },
                )
                .await;
            },
            Err(error) => {
                return rollback_as(transaction, read_error(error)).await;
            },
        };
        let commit = match updated.into_commit() {
            Ok(commit) => commit,
            Err(error) => return rollback_as(transaction, error).await,
        };
        self.commit_acknowledged(transaction, commit).await
    }

    #[tracing::instrument(skip_all)]
    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let expected = tombstone.expected_version();
        let mut transaction = self.begin_mutation().await?;

        let Some(locked) = (match lock_owner_credential(
            &mut transaction,
            &credential_id,
            selector.owner(),
        )
        .await
        {
            Ok(locked) => locked,
            Err(error) => {
                return rollback_as(transaction, read_error(error)).await;
            },
        }) else {
            return rollback_as(transaction, CredentialPersistenceError::NotFound).await;
        };
        let actual = match classify_locked_live(&locked, expected) {
            Ok(actual) => actual,
            Err(error) => return rollback_as(transaction, error).await,
        };
        let next_version = match actual.next_tombstone() {
            Ok(next_version) => next_version,
            Err(error) => return rollback_as(transaction, error).await,
        };

        let updated: Result<Option<CredentialCommitRow>, sqlx::Error> = sqlx::query_as(
            "UPDATE credentials
             SET name = NULL,
                 data = ''::bytea,
                 version = $3,
                 updated_at = CURRENT_TIMESTAMP,
                 expires_at = NULL,
                 reauth_required = FALSE,
                 metadata = '{}',
                 record_state = 'tombstoned',
                 tombstoned_at = CURRENT_TIMESTAMP,
                 refresh_retry_mode = NULL,
                 refresh_retry_not_before = NULL,
                 refresh_retry_phase = NULL,
                 refresh_retry_kind = NULL,
                 refresh_retry_diagnostic_code = NULL
             WHERE id = $1
               AND owner_id = $2
               AND record_state = 'live'
               AND version = $4
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .bind(next_version.get())
        .bind(expected.get())
        .fetch_optional(&mut *transaction)
        .await;

        let updated = match updated {
            Ok(Some(updated)) => updated,
            Ok(None) => {
                return rollback_as(transaction, CredentialPersistenceError::Unavailable).await;
            },
            Err(error) => return rollback_as(transaction, read_error(error)).await,
        };
        let commit = match updated.into_commit() {
            Ok(commit) => commit,
            Err(error) => return rollback_as(transaction, error).await,
        };
        self.commit_acknowledged(transaction, commit).await
    }

    #[tracing::instrument(skip_all)]
    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        let rows: Vec<(String,)> = match state_kind {
            Some(state_kind) => {
                sqlx::query_as(
                    "SELECT id
                     FROM credentials
                     WHERE owner_id = $1
                       AND record_state = 'live'
                       AND state_kind = $2
                     ORDER BY id",
                )
                .bind(owner.as_str())
                .bind(state_kind)
                .fetch_all(&self.pool)
                .await
            },
            None => {
                sqlx::query_as(
                    "SELECT id
                     FROM credentials
                     WHERE owner_id = $1 AND record_state = 'live'
                     ORDER BY id",
                )
                .bind(owner.as_str())
                .fetch_all(&self.pool)
                .await
            },
        }
        .map_err(read_error)?;

        rows.into_iter()
            .map(|(credential_id,)| parse_credential_id(&credential_id))
            .collect()
    }

    #[tracing::instrument(skip_all)]
    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        let rows: Vec<CredentialHeadRow> = match state_kind {
            Some(state_kind) => {
                sqlx::query_as(
                    "SELECT id, name, credential_key, state_kind, state_version,
                            version, material_epoch, created_at, updated_at, expires_at,
                            reauth_required, metadata, record_state, tombstoned_at
                     FROM credentials
                     WHERE owner_id = $1
                       AND record_state = 'live'
                       AND state_kind = $2
                     ORDER BY id",
                )
                .bind(owner.as_str())
                .bind(state_kind)
                .fetch_all(&self.pool)
                .await
            },
            None => {
                sqlx::query_as(
                    "SELECT id, name, credential_key, state_kind, state_version,
                            version, material_epoch, created_at, updated_at, expires_at,
                            reauth_required, metadata, record_state, tombstoned_at
                     FROM credentials
                     WHERE owner_id = $1 AND record_state = 'live'
                     ORDER BY id",
                )
                .bind(owner.as_str())
                .fetch_all(&self.pool)
                .await
            },
        }
        .map_err(read_error)?;

        rows.into_iter()
            .map(CredentialHeadRow::into_stored_head)
            .collect()
    }

    #[tracing::instrument(skip_all)]
    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1
                 FROM credentials
                 WHERE id = $1 AND owner_id = $2 AND record_state = 'live'
             )",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(read_error)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        str::FromStr,
        time::Duration,
        time::{SystemTime, UNIX_EPOCH},
    };

    use nebula_storage_port::{
        CredentialCreate, CredentialReplacement,
        store::{RefreshClaimError, RefreshClaimStore, ReplicaId},
    };
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    use super::*;

    type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

    #[test]
    fn commit_sqlstate_classification_preserves_unknown_outcomes() {
        for code in [
            "40003", "08000", "08003", "08006", "08007", "08P01", "08999",
        ] {
            assert!(
                is_unknown_commit_sqlstate(Some(code)),
                "{code} cannot authoritatively acknowledge a dispatched COMMIT"
            );
        }
        for code in ["23505", "40001", "40P01", "22000"] {
            assert!(
                !is_unknown_commit_sqlstate(Some(code)),
                "{code} is an authoritative database error"
            );
        }
        assert!(!is_unknown_commit_sqlstate(None));
    }

    #[test]
    fn refresh_retry_sql_uses_wall_clock_after_lock_waits() {
        let source = include_str!("postgres.rs");
        let production_source = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("the production adapter must precede its test module")
            .0;
        assert!(production_source.contains("clock_timestamp() AS backend_now"));
        assert!(
            production_source
                .contains("WHEN 3 THEN clock_timestamp() + ($13::BIGINT * INTERVAL '1 second')")
        );
        assert!(
            !production_source
                .contains("WHEN 3 THEN CURRENT_TIMESTAMP + ($13::BIGINT * INTERVAL '1 second')")
        );
        let snapshot_body = production_source
            .split_once("async fn refresh_retry_snapshot(")
            .expect("snapshot method must exist")
            .1
            .split_once("\n    #[tracing::instrument")
            .expect("the following port method must delimit the snapshot body")
            .0;
        assert_eq!(snapshot_body.matches("sqlx::query_as(").count(), 1);
        assert!(snapshot_body.contains("SELECT version, reauth_required, record_state"));
        assert!(snapshot_body.contains("clock_timestamp() AS backend_now"));
        assert!(!snapshot_body.contains("self.get("));
    }

    #[tokio::test]
    async fn curated_refresh_claim_repo_shares_the_admitted_private_pool() -> TestResult {
        let pool = PgPoolOptions::new().connect_lazy("postgres://localhost/nebula")?;
        let store = PgCredentialPersistence::from_admitted_pool(pool);
        let claim_repo = store.refresh_claim_repo();

        // Closing one clone closes the shared pool. A separately-created pool
        // using the same URL would remain open, so this proves the curated
        // seam clones the admitted pool rather than reconstructing a backend.
        store.pool.close().await;

        let result = claim_repo
            .try_claim(
                &CredentialId::new(),
                &ReplicaId::new("same-pool-probe"),
                Duration::from_secs(30),
            )
            .await;
        assert!(
            matches!(result, Err(RefreshClaimError::Storage)),
            "claim adapter must observe closure of the credential store's shared pool"
        );
        Ok(())
    }

    #[tokio::test]
    async fn post_commit_ack_loss_is_unknown_and_is_not_retried() -> TestResult {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => {
                assert!(
                    std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                    "NEBULA_REQUIRE_POSTGRES=1 but DATABASE_URL is absent"
                );
                return Ok(());
            },
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let schema = format!(
            "nebula_credential_commit_fault_{}_{nanos}",
            std::process::id()
        );
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await?;

        let options = PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]);
        let probe = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await?;
        let store = PgCredentialPersistence::connect_with(options).await?;
        let owner = CredentialOwner::from_canonical("tenant-commit-fault");
        let credential_id = CredentialId::new();
        let selector = CredentialSelector::new(owner.clone(), credential_id);
        let created = store
            .create(
                &selector,
                CredentialCreate::new(
                    "provider.api-token".to_owned(),
                    SecretBytes::new(b"before".to_vec()),
                    "active".to_owned(),
                    1,
                    None,
                    None,
                    false,
                    Map::new(),
                ),
            )
            .await?;

        store.lose_next_commit_acknowledgement();
        let outcome = store
            .replace(
                &selector,
                CredentialReplacement::new(
                    created.version(),
                    SecretBytes::new(b"after".to_vec()),
                    "active".to_owned(),
                    2,
                    None,
                    None,
                    false,
                    Map::new(),
                    nebula_storage_port::CredentialMaterialTransition::advance(),
                ),
            )
            .await;
        assert_eq!(
            outcome,
            Err(CredentialPersistenceError::OutcomeUnknown),
            "a lost acknowledgement must never be guessed as success or rollback"
        );

        let physical = store.get(&selector).await?;
        let StoredCredential::Live(live) = physical else {
            panic!("the post-COMMIT fault must leave one durable live row");
        };
        assert_eq!(live.version().get(), 2);
        assert_eq!(live.data().as_ref(), b"after");

        let physical_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM credentials WHERE owner_id = $1 AND id = $2")
                .bind(owner.as_str())
                .bind(credential_id.to_string())
                .fetch_one(&probe)
                .await?;
        assert_eq!(physical_rows, 1, "the adapter must not retry the mutation");

        drop(store);
        probe.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin)
            .await?;
        admin.close().await;
        Ok(())
    }
}
