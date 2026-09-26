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
    CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialIncidentRef,
    CredentialMaterial, CredentialMaterialEpoch, CredentialOperationKind,
    CredentialOperationStatus, CredentialOwner, CredentialPersistence, CredentialPersistenceError,
    CredentialRefreshCursor, CredentialRefreshHorizon, CredentialRefreshPageSize,
    CredentialRefreshSchedule, CredentialRefreshScheduleError, CredentialReplacement,
    CredentialSelector, CredentialTombstone, CredentialVersion, DueCredentialRefresh,
    RefreshRetrySnapshot, SecretBytes, StoredCredential, StoredCredentialHead,
    StoredCredentialOperationalHead, StoredLiveCredential, StoredTombstonedCredential,
};
use serde_json::{Map, Value};
use sqlx::{PgPool, Postgres, Transaction};
#[cfg(test)]
use std::sync::Mutex as StdMutex;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt, num::NonZeroU32, sync::Arc};
#[cfg(test)]
use tokio::sync::Notify;

use super::{
    CredentialStoreStartupError, pending::PgPendingStateStore, refresh_claim::PgRefreshClaimRepo,
    retry_gate, schema::postgres as schema,
};
use crate::migration::setup_postgres_pool_with;

/// Connections a PostgreSQL credential store pools unless told otherwise:
/// SQLx's own default, kept so existing deployments see no change.
pub const DEFAULT_CREDENTIAL_POOL_SIZE: NonZeroU32 = NonZeroU32::new(10).expect("non-zero");

/// PostgreSQL-backed [`CredentialPersistence`].
///
/// The internal pool is cheap to clone, but construction always passes through
/// the schema admission and migration gate.
#[derive(Clone)]
pub struct PgCredentialPersistence {
    pool: PgPool,
    #[cfg(test)]
    lose_next_commit_acknowledgement: Arc<AtomicBool>,
    #[cfg(test)]
    replace_claim_probe_gate: Arc<StdMutex<Option<ReplaceClaimProbeGate>>>,
}

#[cfg(test)]
#[derive(Clone)]
struct ReplaceClaimProbeGate {
    reached: Arc<Notify>,
    resume: Arc<Notify>,
}

/// Read-only due-refresh schedule over an admitted PostgreSQL credential pool.
#[derive(Clone)]
pub struct PgCredentialRefreshSchedule {
    pool: PgPool,
}

impl fmt::Debug for PgCredentialRefreshSchedule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PgCredentialRefreshSchedule")
    }
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
            #[cfg(test)]
            replace_claim_probe_gate: Arc::new(StdMutex::new(None)),
        }
    }

    #[cfg(test)]
    fn arm_replace_claim_probe_gate(&self) -> ReplaceClaimProbeGate {
        let gate = ReplaceClaimProbeGate {
            reached: Arc::new(Notify::new()),
            resume: Arc::new(Notify::new()),
        };
        *self
            .replace_claim_probe_gate
            .lock()
            .expect("replace probe gate mutex") = Some(gate.clone());
        gate
    }

    #[cfg(test)]
    async fn pause_after_replace_claim_probe(&self) {
        let gate = self
            .replace_claim_probe_gate
            .lock()
            .expect("replace probe gate mutex")
            .take();
        if let Some(gate) = gate {
            gate.reached.notify_one();
            gate.resume.notified().await;
        }
    }

    /// Connect, admit the canonical schema, and apply all pending migrations
    /// under the PostgreSQL readiness lock, whose acquisition and release are
    /// bounded.
    ///
    /// # Errors
    ///
    /// Returns a closed, secret-free startup error. URLs and driver messages
    /// are never retained by the error.
    pub async fn connect(url: &str) -> Result<Self, CredentialStoreStartupError> {
        Self::connect_sized(url, DEFAULT_CREDENTIAL_POOL_SIZE).await
    }

    /// [`Self::connect`] with at most `max_connections` pooled connections.
    ///
    /// Every credential admission reads through this pool, so it bounds how
    /// many admissions one process runs against PostgreSQL at once; past it
    /// admissions queue for a connection.
    ///
    /// # Errors
    ///
    /// As [`Self::connect`].
    pub async fn connect_sized(
        url: &str,
        max_connections: NonZeroU32,
    ) -> Result<Self, CredentialStoreStartupError> {
        use std::str::FromStr;

        let options = sqlx::postgres::PgConnectOptions::from_str(url)
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        Self::connect_with_sized(options, max_connections).await
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
        Self::connect_with_sized(options, DEFAULT_CREDENTIAL_POOL_SIZE).await
    }

    /// [`Self::connect_with`] with at most `max_connections` pooled
    /// connections; see [`Self::connect_sized`].
    ///
    /// # Errors
    ///
    /// As [`Self::connect_with`].
    pub async fn connect_with_sized(
        options: sqlx::postgres::PgConnectOptions,
        max_connections: NonZeroU32,
    ) -> Result<Self, CredentialStoreStartupError> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .min_connections(1)
            .max_connections(max_connections.get())
            .connect_with(options)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        setup_postgres_pool_with::<schema::CredentialAdmission>(pool.clone()).await?;

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

    /// Create an encrypted durable pending-state store on this admitted pool.
    #[must_use]
    pub fn pending_state_store(
        &self,
        key_provider: Arc<dyn super::KeyProvider>,
        legacy_keys: Vec<(String, Arc<nebula_crypto::EncryptionKey>)>,
    ) -> PgPendingStateStore {
        PgPendingStateStore::new(self.pool.clone(), key_provider, legacy_keys)
    }

    /// Create the due-refresh schedule adapter on this store's admitted pool.
    #[must_use]
    pub fn refresh_schedule(&self) -> PgCredentialRefreshSchedule {
        PgCredentialRefreshSchedule {
            pool: self.pool.clone(),
        }
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
        "SELECT record_state, version, material_epoch, credential_key, reauth_required
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
    credential_key: String,
    reauth_required: bool,
}

/// A physical record with the claim that governs its use, read in one
/// statement for admission (`get_with_operation_status`).
#[derive(sqlx::FromRow)]
struct CredentialWithStatusRow {
    #[sqlx(flatten)]
    credential: CredentialRow,
    operation_kind: Option<String>,
    operation_sentinel: Option<i16>,
    operation_expires_at: Option<DateTime<Utc>>,
    operation_claim_id: Option<uuid::Uuid>,
    backend_now: DateTime<Utc>,
}

/// Derive a live credential's operation status from its aggregate columns
/// and its claim row (all `None` when it has none), against `now` read in
/// the same statement.
#[expect(
    clippy::too_many_arguments,
    reason = "the columns of one joined row, decoded in one place for every read that projects them"
)]
fn status_from_parts(
    version: i64,
    epoch: i64,
    reauth_required: bool,
    kind: Option<String>,
    sentinel: Option<i16>,
    expires_at: Option<DateTime<Utc>>,
    claim_id: Option<uuid::Uuid>,
    now: DateTime<Utc>,
) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
    let open = || {
        Ok(CredentialOperationStatus::Open {
            version: parse_version(version)?,
            material_epoch: parse_material_epoch(epoch)?,
            reauth_required,
        })
    };
    let (kind, sentinel, expires_at) = match (kind, sentinel, expires_at) {
        (None, None, None) => return open(),
        (Some(kind), Some(sentinel), Some(expires_at)) => (kind, sentinel, expires_at),
        _ => return Err(CredentialPersistenceError::CorruptRecord),
    };
    let operation = CredentialOperationKind::from_wire(&kind)
        .ok_or(CredentialPersistenceError::CorruptRecord)?;
    match sentinel {
        0 if operation == CredentialOperationKind::Refresh || expires_at < now => open(),
        0 | 1 if expires_at >= now => Ok(CredentialOperationStatus::InFlight { operation }),
        1 => Ok(CredentialOperationStatus::ReconciliationRequired {
            operation,
            incident: CredentialIncidentRef::from_uuid(
                claim_id.ok_or(CredentialPersistenceError::CorruptRecord)?,
            ),
        }),
        _ => Err(CredentialPersistenceError::CorruptRecord),
    }
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
    refresh_retry_mode: Option<String>,
    refresh_retry_not_before: Option<DateTime<Utc>>,
    backend_now: DateTime<Utc>,
    metadata: String,
    record_state: String,
    tombstoned_at: Option<DateTime<Utc>>,
    operation_kind: Option<String>,
    operation_sentinel: Option<i16>,
    operation_expires_at: Option<DateTime<Utc>>,
    operation_claim_id: Option<uuid::Uuid>,
}

impl CredentialHeadRow {
    fn operation_status(&self) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        let open = || {
            Ok(CredentialOperationStatus::Open {
                version: parse_version(self.version)?,
                material_epoch: parse_material_epoch(self.material_epoch)?,
                reauth_required: self.reauth_required,
            })
        };
        let (kind, sentinel, expires_at) = match (
            self.operation_kind.as_deref(),
            self.operation_sentinel,
            self.operation_expires_at,
        ) {
            (None, None, None) => return open(),
            (Some(kind), Some(sentinel), Some(expires_at)) => (kind, sentinel, expires_at),
            _ => return Err(CredentialPersistenceError::CorruptRecord),
        };
        let operation = CredentialOperationKind::from_wire(kind)
            .ok_or(CredentialPersistenceError::CorruptRecord)?;
        match sentinel {
            0 if operation == CredentialOperationKind::Refresh || expires_at < self.backend_now => {
                open()
            },
            0 | 1 if expires_at >= self.backend_now => {
                Ok(CredentialOperationStatus::InFlight { operation })
            },
            1 => Ok(CredentialOperationStatus::ReconciliationRequired {
                operation,
                incident: CredentialIncidentRef::from_uuid(
                    self.operation_claim_id
                        .ok_or(CredentialPersistenceError::CorruptRecord)?,
                ),
            }),
            _ => Err(CredentialPersistenceError::CorruptRecord),
        }
    }

    fn into_operational_head(
        self,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        let status = self.operation_status()?;
        let head = self.into_stored_head()?;
        Ok(StoredCredentialOperationalHead::new(head, status))
    }

    fn into_stored_head(self) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        if self.record_state != "live" || self.tombstoned_at.is_some() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        let metadata = decode_metadata(&self.metadata)?;
        validate_name_projection(self.name.as_deref(), &metadata)?;
        let refresh_retry = retry_gate::decode_projection(
            self.refresh_retry_mode,
            self.refresh_retry_not_before,
            self.backend_now,
        )?;
        StoredCredentialHead::new_with_refresh_retry(
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
            refresh_retry,
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
struct RevokedMaterialRow {
    id: String,
    version: i64,
    material_epoch: i64,
    record_state: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tombstoned_at: Option<DateTime<Utc>>,
}

impl RevokedMaterialRow {
    fn into_commit(self) -> Result<CredentialCommit, CredentialPersistenceError> {
        CredentialCommitRow {
            id: self.id,
            version: self.version,
            record_state: self.record_state,
            created_at: self.created_at,
            updated_at: self.updated_at,
            tombstoned_at: self.tombstoned_at,
        }
        .into_commit()
    }
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
impl CredentialRefreshSchedule for PgCredentialRefreshSchedule {
    #[tracing::instrument(skip_all, fields(credential.operation = "scan_due_refresh"))]
    async fn scan_due(
        &self,
        after: Option<&CredentialRefreshCursor>,
        horizon: CredentialRefreshHorizon,
        limit: CredentialRefreshPageSize,
    ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError> {
        let horizon_secs = i64::try_from(horizon.get().as_secs())
            .map_err(|_| CredentialRefreshScheduleError::CorruptRecord)?;
        let (after_expiry, after_id) = after
            .map(|cursor| {
                (
                    Some(cursor.expires_at()),
                    Some(cursor.credential_id().to_string()),
                )
            })
            .unwrap_or((None, None));
        let rows: Vec<(
            String,
            String,
            String,
            DateTime<Utc>,
            Option<String>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "WITH backend_clock AS MATERIALIZED (SELECT clock_timestamp() AS now_at)
             SELECT c.id, c.owner_id, c.credential_key, c.expires_at,
                    c.refresh_retry_mode, c.refresh_retry_not_before, clock.now_at
             FROM credentials AS c CROSS JOIN backend_clock AS clock
             WHERE c.record_state = 'live'
               AND c.expires_at IS NOT NULL
               AND c.reauth_required = FALSE
               AND c.expires_at <= clock.now_at + ($1 * INTERVAL '1 second')
               AND (
                    c.refresh_retry_mode IS NULL
                    OR (c.refresh_retry_mode <> $3
                        AND (c.refresh_retry_mode <> $2
                             OR c.refresh_retry_not_before IS NULL
                             OR c.refresh_retry_not_before <= clock.now_at))
               )
               AND ($4::timestamptz IS NULL OR c.expires_at > $4
                    OR (c.expires_at = $4 AND c.id > $5))
             ORDER BY c.expires_at, c.id
             LIMIT $6",
        )
        .bind(horizon_secs)
        .bind(retry_gate::MODE_NOT_BEFORE)
        .bind(retry_gate::MODE_NEVER)
        .bind(after_expiry)
        .bind(after_id)
        .bind(i64::from(limit.get()))
        .fetch_all(&self.pool)
        .await
        .map_err(|_| CredentialRefreshScheduleError::Unavailable)?;

        rows.into_iter()
            .map(
                |(id, owner, credential_key, expires_at, mode, not_before, observed_at)| {
                    match (mode.as_deref(), not_before) {
                        (None, None) | (Some(retry_gate::MODE_NOT_BEFORE), Some(_)) => {},
                        _ => return Err(CredentialRefreshScheduleError::CorruptRecord),
                    }
                    let credential_id = parse_credential_id(&id)
                        .map_err(|_| CredentialRefreshScheduleError::CorruptRecord)?;
                    Ok(DueCredentialRefresh::new(
                        CredentialSelector::new(
                            CredentialOwner::from_canonical(owner),
                            credential_id,
                        ),
                        credential_key,
                        expires_at,
                        observed_at,
                    ))
                },
            )
            .collect()
    }
}

#[async_trait]
impl CredentialRefreshSchedule for PgCredentialPersistence {
    async fn scan_due(
        &self,
        after: Option<&CredentialRefreshCursor>,
        horizon: CredentialRefreshHorizon,
        limit: CredentialRefreshPageSize,
    ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError> {
        self.refresh_schedule()
            .scan_due(after, horizon, limit)
            .await
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
    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        let row: Option<(
            i64,
            i64,
            bool,
            Option<String>,
            Option<i16>,
            Option<DateTime<Utc>>,
            Option<uuid::Uuid>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT c.version, c.material_epoch, c.reauth_required, claim.operation_kind, \
                        claim.sentinel, claim.expires_at, claim.claim_id, clock_timestamp() \
                 FROM credentials AS c \
                 LEFT JOIN credential_refresh_claims AS claim \
                   ON claim.owner_id = c.owner_id AND claim.credential_id = c.id \
                 WHERE c.id = $1 AND c.owner_id = $2 AND c.record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;
        let Some((version, epoch, reauth_required, kind, sentinel, expires_at, claim_id, now)) =
            row
        else {
            return Err(CredentialPersistenceError::NotFound);
        };
        status_from_parts(
            version,
            epoch,
            reauth_required,
            kind,
            sentinel,
            expires_at,
            claim_id,
            now,
        )
    }

    #[tracing::instrument(skip_all)]
    async fn get_with_operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(StoredCredential, Option<CredentialOperationStatus>), CredentialPersistenceError>
    {
        let row: Option<CredentialWithStatusRow> = sqlx::query_as(
            "SELECT c.id, c.name, c.credential_key, c.data, c.state_kind, c.state_version,
                    c.version, c.material_epoch, c.created_at, c.updated_at, c.expires_at,
                    c.reauth_required, c.metadata, c.record_state, c.tombstoned_at,
                    c.refresh_retry_mode, c.refresh_retry_not_before,
                    c.refresh_retry_phase, c.refresh_retry_kind,
                    c.refresh_retry_diagnostic_code,
                    claim.operation_kind, claim.sentinel AS operation_sentinel,
                    claim.expires_at AS operation_expires_at,
                    claim.claim_id AS operation_claim_id,
                    clock_timestamp() AS backend_now
             FROM credentials AS c
             LEFT JOIN credential_refresh_claims AS claim
               ON claim.owner_id = c.owner_id AND claim.credential_id = c.id
             WHERE c.id = $1 AND c.owner_id = $2",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;
        let Some(row) = row else {
            return Err(CredentialPersistenceError::NotFound);
        };
        let status = if row.credential.record_state == "live" {
            Some(status_from_parts(
                row.credential.version,
                row.credential.material_epoch,
                row.credential.reauth_required,
                row.operation_kind,
                row.operation_sentinel,
                row.operation_expires_at,
                row.operation_claim_id,
                row.backend_now,
            )?)
        } else {
            None
        };
        Ok((row.credential.into_stored()?, status))
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
                    reauth_required, refresh_retry_mode, refresh_retry_not_before,
                    clock_timestamp() AS backend_now,
                    metadata, record_state, tombstoned_at, NULL::text AS operation_kind, \
                    NULL::smallint AS operation_sentinel, NULL::timestamptz AS operation_expires_at, NULL::uuid AS operation_claim_id
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
    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        let row: Option<CredentialHeadRow> = sqlx::query_as(
            "SELECT c.id, c.name, c.credential_key, c.state_kind, c.state_version, \
                    c.version, c.material_epoch, c.created_at, c.updated_at, c.expires_at, \
                    c.reauth_required, c.refresh_retry_mode, c.refresh_retry_not_before, \
                    clock_timestamp() AS backend_now, c.metadata, c.record_state, c.tombstoned_at, \
                    claim.operation_kind, claim.sentinel AS operation_sentinel, \
                    claim.expires_at AS operation_expires_at, claim.claim_id AS operation_claim_id \
             FROM credentials AS c LEFT JOIN credential_refresh_claims AS claim \
               ON claim.owner_id = c.owner_id AND claim.credential_id = c.id \
             WHERE c.id = $1 AND c.owner_id = $2 AND c.record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;
        row.ok_or(CredentialPersistenceError::NotFound)?
            .into_operational_head()
    }

    #[tracing::instrument(skip_all)]
    async fn list_operational_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialOperationalHead>, CredentialPersistenceError> {
        let rows: Vec<CredentialHeadRow> = sqlx::query_as(
            "SELECT c.id, c.name, c.credential_key, c.state_kind, c.state_version, \
                    c.version, c.material_epoch, c.created_at, c.updated_at, c.expires_at, \
                    c.reauth_required, c.refresh_retry_mode, c.refresh_retry_not_before, \
                    clock_timestamp() AS backend_now, c.metadata, c.record_state, c.tombstoned_at, \
                    claim.operation_kind, claim.sentinel AS operation_sentinel, \
                    claim.expires_at AS operation_expires_at, claim.claim_id AS operation_claim_id \
             FROM credentials AS c LEFT JOIN credential_refresh_claims AS claim \
               ON claim.owner_id = c.owner_id AND claim.credential_id = c.id \
             WHERE c.owner_id = $1 AND c.record_state = 'live' \
               AND ($2::text IS NULL OR c.state_kind = $2) ORDER BY c.id",
        )
        .bind(owner.as_str())
        .bind(state_kind)
        .fetch_all(&self.pool)
        .await
        .map_err(read_error)?;
        rows.into_iter()
            .map(CredentialHeadRow::into_operational_head)
            .collect()
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
        let blocking_revoke: Result<Option<(String,)>, sqlx::Error> = sqlx::query_as(
            "SELECT operation_kind FROM credential_refresh_claims \
             WHERE owner_id = $1 AND credential_id = $2 AND operation_kind = 'revoke' \
             FOR UPDATE",
        )
        .bind(selector.owner().as_str())
        .bind(&credential_id)
        .fetch_optional(&mut *transaction)
        .await;
        let blocking_revoke = match blocking_revoke {
            Ok(value) => value.is_some(),
            Err(error) => return rollback_as(transaction, read_error(error)).await,
        };
        #[cfg(test)]
        self.pause_after_replace_claim_probe().await;

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
        let actual_material_epoch = match parse_material_epoch(locked.material_epoch) {
            Ok(epoch) => epoch,
            Err(error) => return rollback_as(transaction, error).await,
        };
        if replacement.material_transition().advances_epoch()
            || locked.reauth_required != replacement.reauth_required()
        {
            let revoke_after_lock: Result<Option<(String,)>, sqlx::Error> = sqlx::query_as(
                "SELECT operation_kind FROM credential_refresh_claims \
                 WHERE owner_id = $1 AND credential_id = $2 AND operation_kind = 'revoke'",
            )
            .bind(selector.owner().as_str())
            .bind(&credential_id)
            .fetch_optional(&mut *transaction)
            .await;
            let revoke_after_lock = match revoke_after_lock {
                Ok(value) => value.is_some(),
                Err(error) => return rollback_as(transaction, read_error(error)).await,
            };
            if blocking_revoke || revoke_after_lock {
                return rollback_as(
                    transaction,
                    CredentialPersistenceError::OperationBlocked {
                        operation: CredentialOperationKind::Revoke,
                    },
                )
                .await;
            }
        }
        if let Some(fence) = replacement.fence() {
            if actual_material_epoch != fence.expected_material_epoch() {
                return rollback_as(
                    transaction,
                    CredentialPersistenceError::VersionConflict { expected, actual },
                )
                .await;
            }
            if locked.credential_key != fence.expected_credential_key() {
                return rollback_as(
                    transaction,
                    CredentialPersistenceError::VersionConflict { expected, actual },
                )
                .await;
            }
        }
        let next_version = match actual.next_live() {
            Ok(next_version) => next_version,
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

        // Material columns are written only for `Advance { Replace }`; every
        // other transition leaves them byte-identical (`$18 = false`).
        let material = replacement.material_transition().material();
        let updated: Result<Option<CredentialCommitRow>, sqlx::Error> = sqlx::query_as(
            "UPDATE credentials
             SET name = $3,
                 data = CASE WHEN $18::BOOLEAN THEN $4::BYTEA ELSE data END,
                 state_kind = CASE WHEN $18::BOOLEAN THEN $5::TEXT ELSE state_kind END,
                 state_version = CASE WHEN $18::BOOLEAN THEN $6::BIGINT ELSE state_version END,
                 version = $7,
                 material_epoch = $8,
                 updated_at = clock_timestamp(),
                 expires_at = CASE WHEN $18::BOOLEAN THEN $9::TIMESTAMPTZ ELSE expires_at END,
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
        .bind(material.map(|material| material.data().as_ref()))
        .bind(material.map(CredentialMaterial::state_kind))
        .bind(material.map(|material| i64::from(material.state_version())))
        .bind(next_version.get())
        .bind(next_material_epoch.get())
        .bind(material.and_then(CredentialMaterial::expires_at))
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
            "WITH mutation_clock AS MATERIALIZED (SELECT clock_timestamp() AS now)
             UPDATE credentials
             SET name = NULL,
                 data = ''::bytea,
                 version = $3,
                 updated_at = mutation_clock.now,
                 expires_at = NULL,
                 reauth_required = FALSE,
                 metadata = '{}',
                 record_state = 'tombstoned',
                 tombstoned_at = mutation_clock.now,
                 refresh_retry_mode = NULL,
                 refresh_retry_not_before = NULL,
                 refresh_retry_phase = NULL,
                 refresh_retry_kind = NULL,
                 refresh_retry_diagnostic_code = NULL
             FROM mutation_clock
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
    async fn tombstone_revoked_material(
        &self,
        selector: &CredentialSelector,
        expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let owner = selector.owner().as_str();
        let mut transaction = self.begin_mutation().await?;

        // Keep the global two-table order claim -> aggregate. The claim lock
        // freezes immutable revoke authority through terminal publication.
        let claim: Option<(String, Option<i64>, i16)> = sqlx::query_as(
            "SELECT operation_kind, observed_material_epoch, sentinel
             FROM credential_refresh_claims
             WHERE owner_id = $1 AND credential_id = $2
             FOR UPDATE",
        )
        .bind(owner)
        .bind(&credential_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(read_error)?;
        if !matches!(
            claim.as_ref(),
            Some((operation, Some(epoch), sentinel))
                if operation == "revoke"
                    && *epoch == expected_material_epoch.get()
                    && *sentinel == 1_i16
        ) {
            return rollback_as(
                transaction,
                CredentialPersistenceError::OperationBlocked {
                    operation: CredentialOperationKind::Revoke,
                },
            )
            .await;
        }

        let row: Option<RevokedMaterialRow> = sqlx::query_as(
            "SELECT id, version, material_epoch, record_state, created_at, updated_at, tombstoned_at
             FROM credentials WHERE owner_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(owner)
        .bind(&credential_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(read_error)?;
        let Some(row) = row else {
            return rollback_as(transaction, CredentialPersistenceError::NotFound).await;
        };
        if row.material_epoch != expected_material_epoch.get() {
            return rollback_as(
                transaction,
                CredentialPersistenceError::OperationBlocked {
                    operation: CredentialOperationKind::Revoke,
                },
            )
            .await;
        }
        if row.record_state == "tombstoned" {
            let commit = row.into_commit()?;
            transaction
                .commit()
                .await
                .map_err(|_| CredentialPersistenceError::OutcomeUnknown)?;
            return Ok(commit);
        }
        if row.record_state != "live" {
            return rollback_as(transaction, CredentialPersistenceError::CorruptRecord).await;
        }
        let current_version = parse_version(row.version)?;
        let next_version = current_version.next_tombstone()?;
        let updated: Option<CredentialCommitRow> = sqlx::query_as(
            "WITH mutation_clock AS MATERIALIZED (SELECT clock_timestamp() AS now)
             UPDATE credentials SET name = NULL, data = ''::bytea, version = $3,
                 updated_at = mutation_clock.now, expires_at = NULL, reauth_required = FALSE,
                 metadata = '{}', record_state = 'tombstoned', tombstoned_at = mutation_clock.now,
                 refresh_retry_mode = NULL, refresh_retry_not_before = NULL,
                 refresh_retry_phase = NULL, refresh_retry_kind = NULL,
                 refresh_retry_diagnostic_code = NULL
             FROM mutation_clock
             WHERE owner_id = $1 AND id = $2 AND record_state = 'live'
               AND material_epoch = $4
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(owner)
        .bind(&credential_id)
        .bind(next_version.get())
        .bind(expected_material_epoch.get())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(read_error)?;
        let commit = updated
            .ok_or(CredentialPersistenceError::CorruptRecord)?
            .into_commit()?;
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
                            reauth_required, refresh_retry_mode, refresh_retry_not_before,
                            clock_timestamp() AS backend_now,
                            metadata, record_state, tombstoned_at, NULL::text AS operation_kind,
                            NULL::smallint AS operation_sentinel, NULL::timestamptz AS operation_expires_at, NULL::uuid AS operation_claim_id
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
                            reauth_required, refresh_retry_mode, refresh_retry_not_before,
                            clock_timestamp() AS backend_now,
                            metadata, record_state, tombstoned_at, NULL::text AS operation_kind,
                            NULL::smallint AS operation_sentinel, NULL::timestamptz AS operation_expires_at, NULL::uuid AS operation_claim_id
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
            production_source.contains("updated_at = clock_timestamp()"),
            "mutation timestamps used for lifecycle projection must observe time after lock waits"
        );
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

        // The clock sample is only meaningful if the same statement also reads
        // the version / reauthentication / record state it is compared against.
        // Assert those are *projected*, rather than pinning the column list
        // verbatim — order and extra projections (`material_epoch`, …) are not
        // part of the guarantee. Narrowing to the projection and stripping `--`
        // comments matters: the body carries a comment mentioning "version",
        // so a substring search over the whole statement would hold even if the
        // column itself were dropped.
        let projection = snapshot_body
            .split_once("SELECT ")
            .expect("the snapshot statement must open with a SELECT projection")
            .1
            .split_once("FROM credentials")
            .expect("the snapshot statement must read from `credentials`")
            .0;
        let projected: Vec<&str> = projection
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(code, _comment)| code))
            .flat_map(|line| line.split(','))
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .collect();
        for column in ["version", "reauth_required", "record_state"] {
            assert!(
                projected.contains(&column),
                "the snapshot statement must project `{column}` alongside its \
                 clock sample; projected: {projected:?}"
            );
        }
        assert!(projected.contains(&"clock_timestamp() AS backend_now"));
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
                &CredentialSelector::new(
                    CredentialOwner::from_canonical("same-pool-owner"),
                    CredentialId::new(),
                ),
                &ReplicaId::new("same-pool-probe"),
                Duration::from_secs(30),
                nebula_storage_port::CredentialOperationIntent::Refresh,
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
                    None,
                    false,
                    Map::new(),
                    nebula_storage_port::CredentialMaterialTransition::advance(
                        nebula_storage_port::MaterialUpdate::Replace(CredentialMaterial::new(
                            SecretBytes::new(b"after".to_vec()),
                            "active".to_owned(),
                            2,
                            None,
                        )),
                    ),
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

    #[tokio::test]
    async fn replace_rechecks_revoke_claim_after_aggregate_lock() -> TestResult {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => {
                assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
                return Ok(());
            },
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };
        let admin = PgPoolOptions::new().connect(&url).await?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let schema = format!("nebula_revoke_replace_race_{}_{nanos}", std::process::id());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await?;
        let options = PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]);
        let store = PgCredentialPersistence::connect_with(options).await?;
        let selector = CredentialSelector::new(
            CredentialOwner::from_canonical("replace-race-owner"),
            CredentialId::new(),
        );
        let created = store
            .create(
                &selector,
                CredentialCreate::new(
                    "oauth".to_owned(),
                    SecretBytes::new(vec![1]),
                    "oauth".to_owned(),
                    1,
                    None,
                    None,
                    false,
                    Map::new(),
                ),
            )
            .await?;
        let gate = store.arm_replace_claim_probe_gate();
        let replacing_store = store.clone();
        let replacing_selector = selector.clone();
        let replace = tokio::spawn(async move {
            replacing_store
                .replace(
                    &replacing_selector,
                    CredentialReplacement::new(
                        created.version(),
                        None,
                        false,
                        Map::new(),
                        nebula_storage_port::CredentialMaterialTransition::advance(
                            nebula_storage_port::MaterialUpdate::Replace(CredentialMaterial::new(
                                SecretBytes::new(vec![2]),
                                "oauth".to_owned(),
                                1,
                                None,
                            )),
                        ),
                    ),
                )
                .await
        });
        gate.reached.notified().await;
        let repo = store.refresh_claim_repo();
        let claim = match repo
            .try_claim(
                &selector,
                &ReplicaId::new("revoke-racer"),
                Duration::from_secs(30),
                nebula_storage_port::CredentialOperationIntent::Revoke {
                    material_epoch: CredentialMaterialEpoch::MIN,
                },
            )
            .await?
        {
            nebula_storage_port::ClaimAttempt::Acquired(claim) => claim,
            other => panic!("revoke must acquire in the controlled gap: {other:?}"),
        };
        gate.resume.notify_one();
        assert!(matches!(
            replace.await?,
            Err(CredentialPersistenceError::OperationBlocked {
                operation: CredentialOperationKind::Revoke,
            })
        ));
        repo.release(claim.token).await?;
        drop(store);
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin)
            .await?;
        admin.close().await;
        Ok(())
    }

    #[tokio::test]
    async fn revoke_finalizer_accepts_display_version_churn_and_refuses_wrong_epoch() -> TestResult
    {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(std::env::VarError::NotPresent) => {
                assert!(std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none());
                return Ok(());
            },
            Err(error) => panic!("DATABASE_URL is set but invalid: {error}"),
        };
        let admin = PgPoolOptions::new().connect(&url).await?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let schema = format!("nebula_revoke_finalizer_{}_{nanos}", std::process::id());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await?;
        let options = PgConnectOptions::from_str(&url)?.options([("search_path", schema.as_str())]);
        let store = PgCredentialPersistence::connect_with(options).await?;
        let selector = CredentialSelector::new(
            CredentialOwner::from_canonical("revoke-finalizer-owner"),
            CredentialId::new(),
        );
        store
            .create(
                &selector,
                CredentialCreate::new(
                    "oauth".to_owned(),
                    SecretBytes::new(vec![1]),
                    "oauth".to_owned(),
                    1,
                    None,
                    None,
                    false,
                    Map::new(),
                ),
            )
            .await?;
        let repo = store.refresh_claim_repo();
        let claim = match repo
            .try_claim(
                &selector,
                &ReplicaId::new("revoke-holder"),
                Duration::from_secs(30),
                nebula_storage_port::CredentialOperationIntent::Revoke {
                    material_epoch: CredentialMaterialEpoch::MIN,
                },
            )
            .await?
        {
            nebula_storage_port::ClaimAttempt::Acquired(claim) => claim,
            other => panic!("revoke claim must be acquired: {other:?}"),
        };
        repo.mark_sentinel(&claim.token).await?;
        store
            .replace(
                &selector,
                CredentialReplacement::new(
                    CredentialVersion::try_from(1_i64)?,
                    None,
                    false,
                    Map::from_iter([("display_revision".to_owned(), Value::from(2))]),
                    nebula_storage_port::CredentialMaterialTransition::preserve(
                        nebula_storage_port::RefreshRetryTransition::Preserve,
                    ),
                ),
            )
            .await?;
        let wrong_epoch = CredentialMaterialEpoch::MIN.next()?;
        assert!(matches!(
            store
                .tombstone_revoked_material(&selector, wrong_epoch)
                .await,
            Err(CredentialPersistenceError::OperationBlocked {
                operation: CredentialOperationKind::Revoke,
            })
        ));
        let commit = store
            .tombstone_revoked_material(&selector, CredentialMaterialEpoch::MIN)
            .await?;
        assert_eq!(commit.version().get(), 3);
        assert!(matches!(
            store.get(&selector).await?,
            StoredCredential::Tombstoned(_)
        ));

        drop(store);
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin)
            .await?;
        admin.close().await;
        Ok(())
    }
}
