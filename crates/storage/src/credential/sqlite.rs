//! SQLite-backed `CredentialPersistence` impl.
//!
//! Persists [`StoredCredential`] rows in the structural `credentials` table at
//! migration `0040_credential_refresh_retry_gate.sql`.
//!
//! - `data` is an opaque `BLOB` — the [`EncryptionLayer`] above us serialises
//!   the AES-256-GCM envelope; we never inspect or decrypt it.
//! - `owner_id` comes only from the mandatory selector, is included in every
//!   row predicate, and is never inferred from metadata.
//! - Every mutation runs under a real `BEGIN IMMEDIATE` transaction, applies
//!   the frozen collision/CAS precedence, and obtains its secret-free
//!   [`CredentialCommit`] from the modifying statement's `RETURNING`
//!   projection. Success is released only after `COMMIT` acknowledgement.
//! - Tombstoning is structural: it clears all live-only values while retaining
//!   the immutable credential/state identity needed by physical binding reads.
//! - Timestamps are stored as `INTEGER` milliseconds-since-epoch (UTC), not
//!   RFC-3339 text, for the same reasons documented in the `RefreshClaimRepo`
//!   SQLite impl (`refresh_claim/sqlite.rs`): integer ordering is unambiguous
//!   for expiry predicates across chrono versions.
//!
//! # Caller contract
//!
//! Composition roots use [`SqliteCredentialPersistence::connect`] so schema
//! admission and migration complete before the adapter becomes reachable.
//!
//! [`EncryptionLayer`]: crate::credential::layer::EncryptionLayer

// budget-justified: cohesive SQLite adapter for one credential aggregate table;
// readiness, structural row mapping, transaction precedence, and the port impl
// intentionally remain adjacent so the lifecycle invariant is reviewable.

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use nebula_core::CredentialId;
use nebula_credential::CredentialDisplay;
use nebula_storage_port::{
    CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialMaterialEpoch,
    CredentialOperationKind, CredentialOperationStatus, CredentialOwner, CredentialPersistence,
    CredentialPersistenceError, CredentialRefreshCursor, CredentialRefreshHorizon,
    CredentialRefreshPageSize, CredentialRefreshSchedule, CredentialRefreshScheduleError,
    CredentialReplacement, CredentialSelector, CredentialTombstone, CredentialVersion,
    DueCredentialRefresh, RefreshRetrySnapshot, SecretBytes, StoredCredential,
    StoredCredentialHead, StoredCredentialOperationalHead, StoredLiveCredential,
    StoredTombstonedCredential,
};
use serde_json::Value;
use sqlx::{Connection, Sqlite, SqlitePool, Transaction};

use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::{
    CredentialStoreStartupError, pending::SqlitePendingStateStore,
    refresh_claim::SqliteRefreshClaimRepo, retry_gate, schema::sqlite as schema,
};
#[cfg(test)]
use crate::migration::SQLITE_MIGRATOR;
use crate::migration::{
    acquire_sqlite_file_setup_guard, acquire_sqlite_memory_setup_guard,
    complete_sqlite_terminal_section, setup_sqlite_connection_with,
};

#[cfg(test)]
#[derive(Debug)]
struct ReadinessTestGate {
    lock_acquired: tokio::sync::Barrier,
    release: tokio::sync::Notify,
}

#[cfg(test)]
impl ReadinessTestGate {
    fn new() -> Self {
        Self {
            lock_acquired: tokio::sync::Barrier::new(2),
            release: tokio::sync::Notify::new(),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TerminalSetupTestGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    pool: tokio::sync::OnceCell<SqlitePool>,
}

#[cfg(test)]
impl TerminalSetupTestGate {
    async fn wait(&self, pool: &SqlitePool) {
        self.pool
            .set(pool.clone())
            .expect("terminal gate is entered once");
        self.entered.notify_one();
        self.release.notified().await;
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestCommitFault {
    armed: AtomicBool,
    injected: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitAcknowledgement {
    Acknowledged,
    AcknowledgementLost,
}

#[derive(Debug, Clone)]
struct SqliteCommitDispatcher {
    #[cfg(test)]
    fault: Arc<TestCommitFault>,
}

impl SqliteCommitDispatcher {
    fn new() -> Self {
        Self {
            #[cfg(test)]
            fault: Arc::new(TestCommitFault::default()),
        }
    }

    async fn dispatch(&self, transaction: Transaction<'static, Sqlite>) -> CommitAcknowledgement {
        #[cfg(test)]
        let conceal_acknowledgement = self.fault.armed.swap(false, Ordering::SeqCst);

        let result = transaction.commit().await;

        #[cfg(test)]
        if conceal_acknowledgement {
            self.fault.injected.fetch_add(1, Ordering::SeqCst);
            return CommitAcknowledgement::AcknowledgementLost;
        }

        match result {
            Ok(()) => CommitAcknowledgement::Acknowledged,
            Err(_) => CommitAcknowledgement::AcknowledgementLost,
        }
    }

    #[cfg(test)]
    fn arm_acknowledgement_loss(&self) {
        self.fault.armed.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn injected_faults(&self) -> usize {
        self.fault.injected.load(Ordering::SeqCst)
    }
}

/// SQLite-backed [`CredentialPersistence`].
///
/// Wraps a `SqlitePool`. Cheap to clone (pool is `Arc`-backed).
/// The pool must already satisfy the canonical migration-0040 schema.
#[derive(Clone, Debug)]
pub struct SqliteCredentialPersistence {
    pool: SqlitePool,
    commit_dispatcher: SqliteCommitDispatcher,
    #[cfg(test)]
    precommit_fault: Arc<AtomicBool>,
}

/// Read-only due-refresh schedule over an admitted SQLite credential pool.
#[derive(Clone, Debug)]
pub struct SqliteCredentialRefreshSchedule {
    pool: SqlitePool,
}

impl SqliteCredentialPersistence {
    fn from_ready_pool(pool: SqlitePool) -> Self {
        Self {
            pool,
            commit_dispatcher: SqliteCommitDispatcher::new(),
            #[cfg(test)]
            precommit_fault: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(crate) fn arm_post_commit_outcome_unknown(&self) {
        self.commit_dispatcher.arm_acknowledgement_loss();
    }

    #[cfg(test)]
    fn injected_post_commit_faults(&self) -> usize {
        self.commit_dispatcher.injected_faults()
    }

    #[cfg(test)]
    fn arm_precommit_failure(&self) {
        self.precommit_fault.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn inject_precommit_failure(
        &self,
        result: Result<CredentialCommit, CredentialPersistenceError>,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        if result.is_ok() && self.precommit_fault.swap(false, Ordering::SeqCst) {
            Err(CredentialPersistenceError::Unavailable)
        } else {
            result
        }
    }

    async fn finish_write(
        &self,
        transaction: Transaction<'static, Sqlite>,
        result: Result<CredentialCommit, CredentialPersistenceError>,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        match result {
            Ok(commit) => match self.commit_dispatcher.dispatch(transaction).await {
                CommitAcknowledgement::Acknowledged => Ok(commit),
                CommitAcknowledgement::AcknowledgementLost => {
                    Err(CredentialPersistenceError::OutcomeUnknown)
                },
            },
            Err(error) => {
                transaction.rollback().await.map_err(unavailable)?;
                Err(error)
            },
        }
    }

    /// Open a supported SQLite database, admit its canonical schema, apply all
    /// pending migrations, and return a ready store.
    ///
    /// `url` is a SQLite connection string — a file URL
    /// (`sqlite://path/to/credentials.db`), a bare path, or
    /// `sqlite::memory:` for an ephemeral store. A local file is protected by
    /// a serialized outer file lock while an
    /// immutable read-only preflight, canonical SQLx migration, and postflight
    /// run. The exact `sqlite::memory:` form is serialized process-wide and
    /// uses one physical connection during readiness.
    /// Once terminal setup begins, it retains its connection and setup guard
    /// through migration and postflight even if the caller cancels this future.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialStoreStartupError::UnsupportedSchemaVersion`] for a
    /// reachable but unsupported schema, or
    /// [`CredentialStoreStartupError::Unavailable`] for connection, lock, or
    /// migration failure. Neither error retains the URL or a driver message.
    pub async fn connect(url: &str) -> Result<Self, CredentialStoreStartupError> {
        use std::str::FromStr;

        let options = sqlx::sqlite::SqliteConnectOptions::from_str(url)
            .map_err(|_| CredentialStoreStartupError::Unavailable)?
            .create_if_missing(true);

        if url == "sqlite::memory:" {
            return Self::connect_memory_options(
                options,
                #[cfg(test)]
                None,
            )
            .await;
        }
        Self::connect_file_options(options).await
    }

    /// Open the supported single-connection `sqlite::memory:` store with the
    /// canonical migration history applied.
    ///
    /// Each call owns an isolated SQLite database. The pool is permanently
    /// capped at one physical connection because separate connections to this
    /// exact SQLite form would otherwise observe separate databases.
    ///
    /// # Errors
    ///
    /// Returns a closed, secret-free startup error if readiness fails.
    pub async fn connect_memory() -> Result<Self, CredentialStoreStartupError> {
        Self::connect("sqlite::memory:").await
    }

    /// Create the refresh-claim adapter on this store's admitted private pool.
    ///
    /// This is the supported composition seam for pairing owner-bound
    /// credential persistence with durable cross-replica refresh
    /// coordination. It clones the pool handle without exposing raw SQL
    /// authority, so both adapters share one schema lifecycle and one SQLite
    /// database.
    #[must_use]
    pub fn refresh_claim_repo(&self) -> SqliteRefreshClaimRepo {
        SqliteRefreshClaimRepo::new(self.pool.clone())
    }

    /// Create an encrypted durable pending-state store on this admitted pool.
    #[must_use]
    pub fn pending_state_store(
        &self,
        key_provider: Arc<dyn super::KeyProvider>,
        legacy_keys: Vec<(String, Arc<nebula_crypto::EncryptionKey>)>,
    ) -> SqlitePendingStateStore {
        SqlitePendingStateStore::new(self.pool.clone(), key_provider, legacy_keys)
    }

    /// Create the due-refresh schedule adapter on this store's admitted pool.
    #[must_use]
    pub fn refresh_schedule(&self) -> SqliteCredentialRefreshSchedule {
        SqliteCredentialRefreshSchedule {
            pool: self.pool.clone(),
        }
    }

    async fn connect_memory_options(
        options: sqlx::sqlite::SqliteConnectOptions,
        #[cfg(test)] terminal_gate: Option<Arc<TerminalSetupTestGate>>,
    ) -> Result<Self, CredentialStoreStartupError> {
        let readiness = acquire_sqlite_memory_setup_guard().await?;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        let mut connection = pool
            .acquire()
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        complete_sqlite_terminal_section(readiness, async move {
            #[cfg(test)]
            if let Some(gate) = terminal_gate {
                gate.wait(&pool).await;
            }
            setup_sqlite_connection_with::<schema::CredentialAdmission>(&mut connection).await?;
            drop(connection);
            Ok(Self::from_ready_pool(pool))
        })
        .await
    }

    async fn connect_file_options(
        options: sqlx::sqlite::SqliteConnectOptions,
    ) -> Result<Self, CredentialStoreStartupError> {
        #[cfg(test)]
        return Self::connect_file_options_inner(options, None, None).await;
        #[cfg(not(test))]
        return Self::connect_file_options_inner(options).await;
    }

    #[cfg(test)]
    async fn connect_file_options_with_gate(
        options: sqlx::sqlite::SqliteConnectOptions,
        gate: &ReadinessTestGate,
    ) -> Result<Self, CredentialStoreStartupError> {
        Self::connect_file_options_inner(options, Some(gate), None).await
    }

    async fn connect_file_options_inner(
        options: sqlx::sqlite::SqliteConnectOptions,
        #[cfg(test)] gate: Option<&ReadinessTestGate>,
        #[cfg(test)] terminal_gate: Option<Arc<TerminalSetupTestGate>>,
    ) -> Result<Self, CredentialStoreStartupError> {
        let path = options.get_filename().to_owned();
        let lock = acquire_sqlite_file_setup_guard(path.clone()).await?;
        #[cfg(test)]
        if let Some(gate) = gate {
            gate.lock_acquired.wait().await;
            gate.release.notified().await;
        }
        let (locked_file_len, has_sidecar) = lock.initial_file_state();

        if locked_file_len == 0 {
            if has_sidecar {
                return Err(CredentialStoreStartupError::Unavailable);
            }
        } else {
            let probe_options = options
                .clone()
                .create_if_missing(false)
                .read_only(true)
                .immutable(true);
            let mut probe = sqlx::SqliteConnection::connect_with(&probe_options)
                .await
                .map_err(|_| CredentialStoreStartupError::Unavailable)?;
            schema::admit(&mut probe).await?;
            probe
                .close()
                .await
                .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        }

        let writable_options = options
            .create_if_missing(true)
            .read_only(false)
            .immutable(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .min_connections(1)
            .connect_with(writable_options)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        let mut connection = pool
            .acquire()
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        complete_sqlite_terminal_section(lock, async move {
            #[cfg(test)]
            if let Some(gate) = terminal_gate {
                gate.wait(&pool).await;
            }
            setup_sqlite_connection_with::<schema::CredentialAdmission>(&mut connection).await?;
            drop(connection);
            Ok(Self::from_ready_pool(pool))
        })
        .await
    }
}

#[cfg(test)]
#[async_trait]
impl super::CredentialPersistenceConformance for SqliteCredentialPersistence {
    async fn force_live_version_for_conformance(
        &self,
        selector: &CredentialSelector,
        version: CredentialVersion,
    ) -> Result<(), CredentialPersistenceError> {
        if !version.is_live() {
            return Err(CredentialPersistenceError::CorruptRecord);
        }
        let updated = sqlx::query(
            "UPDATE credentials SET version = ?1
             WHERE id = ?2 AND owner_id = ?3 AND record_state = 'live'",
        )
        .bind(version.get())
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .execute(&self.pool)
        .await
        .map_err(unavailable)?;
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
            "UPDATE credentials SET material_epoch = ?1
             WHERE id = ?2 AND owner_id = ?3 AND record_state = 'live'",
        )
        .bind(material_epoch.get())
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .execute(&self.pool)
        .await
        .map_err(unavailable)?;
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
             WHERE id = ?1 AND owner_id = ?2 AND record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .execute(&self.pool)
        .await
        .map_err(unavailable)?;
        if updated.rows_affected() != 1 {
            return Err(CredentialPersistenceError::NotFound);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "sqlite_tests.rs"]
mod tests;

// ── helpers ──────────────────────────────────────────────────────────────────

fn unavailable(_: sqlx::Error) -> CredentialPersistenceError {
    CredentialPersistenceError::Unavailable
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

/// Convert a millisecond-since-epoch `INTEGER` column back to `DateTime<Utc>`.
fn millis_to_utc(ms: i64) -> Result<DateTime<Utc>, CredentialPersistenceError> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .ok_or(CredentialPersistenceError::CorruptRecord)
}

fn stored_version(value: i64) -> Result<CredentialVersion, CredentialPersistenceError> {
    CredentialVersion::try_from(value).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn stored_material_epoch(
    value: i64,
) -> Result<CredentialMaterialEpoch, CredentialPersistenceError> {
    CredentialMaterialEpoch::try_from(value).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn stored_credential_id(value: &str) -> Result<CredentialId, CredentialPersistenceError> {
    value
        .parse()
        .map_err(|_| CredentialPersistenceError::CorruptRecord)
}

/// Serialize the metadata map to a JSON string for the `TEXT` column.
fn meta_to_json(
    meta: &serde_json::Map<String, Value>,
) -> Result<String, CredentialPersistenceError> {
    serde_json::to_string(meta).map_err(|_| CredentialPersistenceError::Unavailable)
}

/// Deserialize the `TEXT` metadata column back to a map.
fn json_to_meta(s: &str) -> Result<serde_json::Map<String, Value>, CredentialPersistenceError> {
    serde_json::from_str(s).map_err(|_| CredentialPersistenceError::CorruptRecord)
}

fn validate_name_projection(
    name: Option<&str>,
    metadata: &serde_json::Map<String, Value>,
) -> Result<(), CredentialPersistenceError> {
    let projected_name = match metadata.get("display") {
        None => None,
        Some(display @ Value::Object(_)) => {
            let display: CredentialDisplay = serde_json::from_value(display.clone())
                .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
            if name != display.display_name.as_deref() {
                return Err(CredentialPersistenceError::CorruptRecord);
            }
            return Ok(());
        },
        Some(_) => return Err(CredentialPersistenceError::CorruptRecord),
    };
    if name != projected_name {
        return Err(CredentialPersistenceError::CorruptRecord);
    }
    Ok(())
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
    material_epoch: i64,
    created_at: i64,
    updated_at: i64,
    expires_at: Option<i64>,
    reauth_required: i64,
    metadata: String,
    record_state: String,
    tombstoned_at: Option<i64>,
    refresh_retry_mode: Option<String>,
    refresh_retry_not_before: Option<i64>,
    refresh_retry_phase: Option<String>,
    refresh_retry_kind: Option<String>,
    refresh_retry_diagnostic_code: Option<String>,
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
    material_epoch: i64,
    created_at: i64,
    updated_at: i64,
    expires_at: Option<i64>,
    reauth_required: i64,
    refresh_retry_mode: Option<String>,
    refresh_retry_not_before: Option<i64>,
    backend_now: i64,
    metadata: String,
    operation_kind: Option<String>,
    operation_sentinel: Option<i64>,
    operation_expires_at: Option<i64>,
}

impl CredentialHeadRow {
    fn operation_status(&self) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        let open = || {
            Ok(CredentialOperationStatus::Open {
                version: stored_version(self.version)?,
                material_epoch: stored_material_epoch(self.material_epoch)?,
                reauth_required: match self.reauth_required {
                    0 => false,
                    1 => true,
                    _ => return Err(CredentialPersistenceError::CorruptRecord),
                },
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
            1 => Ok(CredentialOperationStatus::ReconciliationRequired { operation }),
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
        let reauth_required = match self.reauth_required {
            0 => false,
            1 => true,
            _ => return Err(CredentialPersistenceError::CorruptRecord),
        };
        let metadata = json_to_meta(&self.metadata)?;
        validate_name_projection(self.name.as_deref(), &metadata)?;
        let refresh_retry = retry_gate::decode_projection(
            self.refresh_retry_mode,
            self.refresh_retry_not_before
                .map(millis_to_utc)
                .transpose()?,
            millis_to_utc(self.backend_now)?,
        )?;
        StoredCredentialHead::new_with_refresh_retry(
            stored_credential_id(&self.id)?,
            self.name,
            self.credential_key,
            self.state_kind,
            u32::try_from(self.state_version)
                .map_err(|_| CredentialPersistenceError::CorruptRecord)?,
            stored_version(self.version)?,
            stored_material_epoch(self.material_epoch)?,
            millis_to_utc(self.created_at)?,
            millis_to_utc(self.updated_at)?,
            self.expires_at.map(millis_to_utc).transpose()?,
            reauth_required,
            refresh_retry,
            metadata,
        )
    }
}

impl CredentialRow {
    fn into_stored(self) -> Result<StoredCredential, CredentialPersistenceError> {
        let credential_id = stored_credential_id(&self.id)?;
        let state_version = u32::try_from(self.state_version)
            .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
        let version = stored_version(self.version)?;
        let material_epoch = stored_material_epoch(self.material_epoch)?;
        let created_at = millis_to_utc(self.created_at)?;
        let updated_at = millis_to_utc(self.updated_at)?;
        let refresh_retry_gate = retry_gate::decode_gate(
            self.refresh_retry_mode,
            self.refresh_retry_not_before
                .map(millis_to_utc)
                .transpose()?,
            self.refresh_retry_phase,
            self.refresh_retry_kind,
            self.refresh_retry_diagnostic_code,
        )?;

        match self.record_state.as_str() {
            "live" => {
                if self.tombstoned_at.is_some() {
                    return Err(CredentialPersistenceError::CorruptRecord);
                }
                let reauth_required = match self.reauth_required {
                    0 => false,
                    1 => true,
                    _ => return Err(CredentialPersistenceError::CorruptRecord),
                };
                let metadata = json_to_meta(&self.metadata)?;
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
                    created_at,
                    updated_at,
                    self.expires_at.map(millis_to_utc).transpose()?,
                    reauth_required,
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
                    || self.reauth_required != 0
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
                        created_at,
                        updated_at,
                        millis_to_utc(tombstoned_at)?,
                    ),
                ))
            },
            _ => Err(CredentialPersistenceError::CorruptRecord),
        }
    }
}

#[derive(sqlx::FromRow)]
struct CredentialCommitRow {
    id: String,
    version: i64,
    record_state: String,
    created_at: i64,
    updated_at: i64,
    tombstoned_at: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct RevokedMaterialRow {
    id: String,
    version: i64,
    material_epoch: i64,
    record_state: String,
    created_at: i64,
    updated_at: i64,
    tombstoned_at: Option<i64>,
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

impl CredentialCommitRow {
    fn into_commit(self) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = stored_credential_id(&self.id)?;
        let version = stored_version(self.version)?;
        let created_at = millis_to_utc(self.created_at)?;
        let updated_at = millis_to_utc(self.updated_at)?;
        match self.record_state.as_str() {
            "live" if self.tombstoned_at.is_none() => {
                CredentialCommit::live(credential_id, version, created_at, updated_at)
            },
            "tombstoned" => {
                let tombstoned_at = self
                    .tombstoned_at
                    .ok_or(CredentialPersistenceError::CorruptRecord)?;
                Ok(CredentialCommit::tombstoned(
                    credential_id,
                    version,
                    created_at,
                    updated_at,
                    millis_to_utc(tombstoned_at)?,
                ))
            },
            _ => Err(CredentialPersistenceError::CorruptRecord),
        }
    }
}

#[derive(sqlx::FromRow)]
struct CredentialLifecycleRow {
    version: i64,
    material_epoch: i64,
    credential_key: String,
    record_state: String,
    reauth_required: i64,
}

#[derive(sqlx::FromRow)]
struct RefreshRetrySnapshotRow {
    version: i64,
    material_epoch: i64,
    reauth_required: i64,
    record_state: String,
    refresh_retry_mode: Option<String>,
    refresh_retry_not_before: Option<i64>,
    refresh_retry_phase: Option<String>,
    refresh_retry_kind: Option<String>,
    refresh_retry_diagnostic_code: Option<String>,
    backend_now: i64,
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
        let material_epoch = stored_material_epoch(self.material_epoch)?;
        let reauth_required = match self.reauth_required {
            0 => false,
            1 => true,
            _ => return Err(CredentialPersistenceError::CorruptRecord),
        };
        let gate = retry_gate::decode_gate(
            self.refresh_retry_mode,
            self.refresh_retry_not_before
                .map(millis_to_utc)
                .transpose()?,
            self.refresh_retry_phase,
            self.refresh_retry_kind,
            self.refresh_retry_diagnostic_code,
        )?;
        let admission = retry_gate::evaluate_gate(gate.as_ref(), millis_to_utc(self.backend_now)?)?;
        Ok(RefreshRetrySnapshot::new(
            version,
            material_epoch,
            reauth_required,
            admission,
        ))
    }
}

// ── CredentialPersistence impl ──────────────────────────────────────────────────────

#[async_trait]
impl CredentialRefreshSchedule for SqliteCredentialRefreshSchedule {
    #[tracing::instrument(skip_all, fields(credential.operation = "scan_due_refresh"))]
    async fn scan_due(
        &self,
        after: Option<&CredentialRefreshCursor>,
        horizon: CredentialRefreshHorizon,
        limit: CredentialRefreshPageSize,
    ) -> Result<Vec<DueCredentialRefresh>, CredentialRefreshScheduleError> {
        let horizon_ms = i64::try_from(horizon.get().as_millis())
            .map_err(|_| CredentialRefreshScheduleError::CorruptRecord)?;
        let (after_expiry, after_id) = after
            .map(|cursor| {
                (
                    Some(cursor.expires_at().timestamp_millis()),
                    Some(cursor.credential_id().to_string()),
                )
            })
            .unwrap_or((None, None));
        let rows: Vec<(
            String,
            String,
            String,
            i64,
            Option<String>,
            Option<i64>,
            i64,
        )> = sqlx::query_as(
            "WITH backend_clock AS (
                 SELECT (CAST(strftime('%s', 'now') AS INTEGER) * 1000
                         + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS now_ms
             )
             SELECT c.id, c.owner_id, c.credential_key, c.expires_at,
                    c.refresh_retry_mode, c.refresh_retry_not_before, clock.now_ms
             FROM credentials AS c CROSS JOIN backend_clock AS clock
             WHERE c.record_state = 'live'
               AND c.expires_at IS NOT NULL
               AND c.reauth_required = 0
               AND c.expires_at <= clock.now_ms + ?1
               AND (
                    c.refresh_retry_mode IS NULL
                    OR (c.refresh_retry_mode <> ?3
                        AND (c.refresh_retry_mode <> ?2
                             OR c.refresh_retry_not_before IS NULL
                             OR c.refresh_retry_not_before <= clock.now_ms))
               )
               AND (?4 IS NULL OR c.expires_at > ?4
                    OR (c.expires_at = ?4 AND c.id > ?5))
             ORDER BY c.expires_at, c.id
             LIMIT ?6",
        )
        .bind(horizon_ms)
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
                    let credential_id = stored_credential_id(&id)
                        .map_err(|_| CredentialRefreshScheduleError::CorruptRecord)?;
                    let expires_at = millis_to_utc(expires_at)
                        .map_err(|_| CredentialRefreshScheduleError::CorruptRecord)?;
                    let observed_at = millis_to_utc(observed_at)
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
impl CredentialRefreshSchedule for SqliteCredentialPersistence {
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
impl CredentialPersistence for SqliteCredentialPersistence {
    #[tracing::instrument(skip_all, fields(credential.operation = "get"))]
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version, version, material_epoch, \
             created_at, updated_at, expires_at, reauth_required, metadata, \
             record_state, tombstoned_at, refresh_retry_mode, \
             refresh_retry_not_before, refresh_retry_phase, refresh_retry_kind, \
             refresh_retry_diagnostic_code \
             FROM credentials WHERE id = ?1 AND owner_id = ?2",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;

        match row {
            Some(row) => row.into_stored(),
            None => Err(CredentialPersistenceError::NotFound),
        }
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "operation_status"))]
    async fn operation_status(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialOperationStatus, CredentialPersistenceError> {
        let row: Option<(i64, i64, i64, Option<String>, Option<i64>, Option<i64>, i64)> =
            sqlx::query_as(
                "SELECT c.version, c.material_epoch, c.reauth_required, claim.operation_kind, \
                        claim.sentinel, claim.expires_at, \
                        (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                         + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) \
                 FROM credentials AS c \
                 LEFT JOIN credential_refresh_claims AS claim \
                   ON claim.owner_id = c.owner_id AND claim.credential_id = c.id \
                 WHERE c.id = ?1 AND c.owner_id = ?2 AND c.record_state = 'live'",
            )
            .bind(selector.credential_id().to_string())
            .bind(selector.owner().as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(read_error)?;
        let Some((version, epoch, reauth_raw, kind, sentinel, expires_at, now)) = row else {
            return Err(CredentialPersistenceError::NotFound);
        };
        let reauth_required = match reauth_raw {
            0 => false,
            1 => true,
            _ => return Err(CredentialPersistenceError::CorruptRecord),
        };
        let open = || {
            Ok(CredentialOperationStatus::Open {
                version: stored_version(version)?,
                material_epoch: stored_material_epoch(epoch)?,
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
            1 => Ok(CredentialOperationStatus::ReconciliationRequired { operation }),
            _ => Err(CredentialPersistenceError::CorruptRecord),
        }
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "refresh_snapshot"))]
    async fn refresh_retry_snapshot(
        &self,
        selector: &CredentialSelector,
    ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
        let row: Option<RefreshRetrySnapshotRow> = sqlx::query_as(
            "SELECT version, material_epoch, reauth_required, record_state, refresh_retry_mode, \
                    refresh_retry_not_before, refresh_retry_phase, \
                    refresh_retry_kind, refresh_retry_diagnostic_code, \
                    (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                     + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS backend_now \
             FROM credentials WHERE id = ?1 AND owner_id = ?2",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;

        row.ok_or(CredentialPersistenceError::NotFound)?
            .into_snapshot()
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "get_head"))]
    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let row: Option<CredentialHeadRow> = sqlx::query_as(
            "SELECT id, name, credential_key, state_kind, state_version, version, material_epoch, \
             created_at, updated_at, expires_at, reauth_required, \
             refresh_retry_mode, refresh_retry_not_before, \
             (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
              + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS backend_now, metadata, NULL AS operation_kind, \
             NULL AS operation_sentinel, NULL AS operation_expires_at \
             FROM credentials \
             WHERE id = ?1 AND owner_id = ?2 AND record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;

        match row {
            Some(row) => row.into_stored_head(),
            None => Err(CredentialPersistenceError::NotFound),
        }
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "get_operational_head"))]
    async fn get_operational_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialOperationalHead, CredentialPersistenceError> {
        let row: Option<CredentialHeadRow> = sqlx::query_as(
            "SELECT c.id, c.name, c.credential_key, c.state_kind, c.state_version, \
             c.version, c.material_epoch, c.created_at, c.updated_at, c.expires_at, \
             c.reauth_required, c.refresh_retry_mode, c.refresh_retry_not_before, \
             (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
              + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS backend_now, \
             c.metadata, claim.operation_kind, claim.sentinel AS operation_sentinel, \
             claim.expires_at AS operation_expires_at \
             FROM credentials AS c LEFT JOIN credential_refresh_claims AS claim \
               ON claim.owner_id = c.owner_id AND claim.credential_id = c.id \
             WHERE c.id = ?1 AND c.owner_id = ?2 AND c.record_state = 'live'",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;
        row.ok_or(CredentialPersistenceError::NotFound)?
            .into_operational_head()
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "list_operational_heads"))]
    async fn list_operational_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialOperationalHead>, CredentialPersistenceError> {
        let rows: Vec<CredentialHeadRow> = sqlx::query_as(
            "SELECT c.id, c.name, c.credential_key, c.state_kind, c.state_version, \
             c.version, c.material_epoch, c.created_at, c.updated_at, c.expires_at, \
             c.reauth_required, c.refresh_retry_mode, c.refresh_retry_not_before, \
             (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
              + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS backend_now, \
             c.metadata, claim.operation_kind, claim.sentinel AS operation_sentinel, \
             claim.expires_at AS operation_expires_at \
             FROM credentials AS c LEFT JOIN credential_refresh_claims AS claim \
               ON claim.owner_id = c.owner_id AND claim.credential_id = c.id \
             WHERE c.owner_id = ?1 AND c.record_state = 'live' \
               AND (?2 IS NULL OR c.state_kind = ?2) ORDER BY c.id",
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

    #[tracing::instrument(skip_all, fields(credential.operation = "create"))]
    async fn create(
        &self,
        selector: &CredentialSelector,
        create: CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        validate_name_projection(create.name(), create.metadata())?;
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(unavailable)?;
        let result = Self::create_in_transaction(&mut transaction, selector, &create).await;
        #[cfg(test)]
        let result = self.inject_precommit_failure(result);
        self.finish_write(transaction, result).await
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "replace"))]
    async fn replace(
        &self,
        selector: &CredentialSelector,
        replacement: CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        validate_name_projection(replacement.name(), replacement.metadata())?;
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(unavailable)?;
        let result = Self::replace_in_transaction(&mut transaction, selector, &replacement).await;
        #[cfg(test)]
        let result = self.inject_precommit_failure(result);
        self.finish_write(transaction, result).await
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "tombstone"))]
    async fn tombstone(
        &self,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(unavailable)?;
        let result = Self::tombstone_in_transaction(&mut transaction, selector, tombstone).await;
        #[cfg(test)]
        let result = self.inject_precommit_failure(result);
        self.finish_write(transaction, result).await
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "tombstone_revoked_material"))]
    async fn tombstone_revoked_material(
        &self,
        selector: &CredentialSelector,
        expected_material_epoch: CredentialMaterialEpoch,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(unavailable)?;
        let credential_id = selector.credential_id().to_string();
        let claim: Option<(String, Option<i64>, i64)> = sqlx::query_as(
            "SELECT operation_kind, observed_material_epoch, sentinel
             FROM credential_refresh_claims
             WHERE owner_id = ?1 AND credential_id = ?2",
        )
        .bind(selector.owner().as_str())
        .bind(&credential_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(read_error)?;
        if !matches!(
            claim.as_ref(),
            Some((operation, Some(epoch), sentinel))
                if operation == "revoke"
                    && *epoch == expected_material_epoch.get()
                    && *sentinel == 1_i64
        ) {
            return self
                .finish_write(
                    transaction,
                    Err(CredentialPersistenceError::OperationBlocked {
                        operation: CredentialOperationKind::Revoke,
                    }),
                )
                .await;
        }
        let row: Option<RevokedMaterialRow> = sqlx::query_as(
            "SELECT id, version, material_epoch, record_state, created_at, updated_at, tombstoned_at
             FROM credentials WHERE owner_id = ?1 AND id = ?2",
        )
        .bind(selector.owner().as_str())
        .bind(&credential_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(read_error)?;
        let Some(row) = row else {
            return self
                .finish_write(transaction, Err(CredentialPersistenceError::NotFound))
                .await;
        };
        if row.material_epoch != expected_material_epoch.get() {
            return self
                .finish_write(
                    transaction,
                    Err(CredentialPersistenceError::OperationBlocked {
                        operation: CredentialOperationKind::Revoke,
                    }),
                )
                .await;
        }
        if row.record_state == "tombstoned" {
            let commit = row.into_commit()?;
            transaction.commit().await.map_err(unavailable)?;
            return Ok(commit);
        }
        if row.record_state != "live" {
            return self
                .finish_write(transaction, Err(CredentialPersistenceError::CorruptRecord))
                .await;
        }
        let next_version = stored_version(row.version)?.next_tombstone()?;
        let now_ms = Utc::now().timestamp_millis();
        let result = sqlx::query_as::<_, CredentialCommitRow>(
            "UPDATE credentials SET name = NULL, data = zeroblob(0), version = ?3,
                 updated_at = ?4, expires_at = NULL, reauth_required = 0, metadata = '{}',
                 record_state = 'tombstoned', tombstoned_at = ?4,
                 refresh_retry_mode = NULL, refresh_retry_not_before = NULL,
                 refresh_retry_phase = NULL, refresh_retry_kind = NULL,
                 refresh_retry_diagnostic_code = NULL
             WHERE owner_id = ?1 AND id = ?2 AND record_state = 'live'
               AND material_epoch = ?5
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(selector.owner().as_str())
        .bind(&credential_id)
        .bind(next_version.get())
        .bind(now_ms)
        .bind(expected_material_epoch.get())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(read_error)?
        .ok_or(CredentialPersistenceError::CorruptRecord)?
        .into_commit();
        self.finish_write(transaction, result).await
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "list"))]
    async fn list(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
        let ids: Vec<(String,)> = match state_kind {
            Some(kind) => sqlx::query_as(
                "SELECT id FROM credentials \
                 WHERE owner_id = ?1 AND state_kind = ?2 AND record_state = 'live' \
                 ORDER BY id",
            )
            .bind(owner.as_str())
            .bind(kind)
            .fetch_all(&self.pool)
            .await
            .map_err(read_error)?,
            None => sqlx::query_as(
                "SELECT id FROM credentials \
                 WHERE owner_id = ?1 AND record_state = 'live' ORDER BY id",
            )
            .bind(owner.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(read_error)?,
        };
        ids.into_iter()
            .map(|(id,)| stored_credential_id(&id))
            .collect()
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "list_heads"))]
    async fn list_heads(
        &self,
        owner: &CredentialOwner,
        state_kind: Option<&str>,
    ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
        let rows: Vec<CredentialHeadRow> = match state_kind {
            Some(kind) => sqlx::query_as(
                "SELECT id, name, credential_key, state_kind, state_version, version, material_epoch, \
                 created_at, updated_at, expires_at, reauth_required, \
                 refresh_retry_mode, refresh_retry_not_before, \
                 (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                  + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS backend_now, metadata, NULL AS operation_kind, \
                 NULL AS operation_sentinel, NULL AS operation_expires_at \
                 FROM credentials \
                 WHERE owner_id = ?1 AND state_kind = ?2 AND record_state = 'live' \
                 ORDER BY id",
            )
            .bind(owner.as_str())
            .bind(kind)
            .fetch_all(&self.pool)
            .await
            .map_err(read_error)?,
            None => sqlx::query_as(
                "SELECT id, name, credential_key, state_kind, state_version, version, material_epoch, \
                 created_at, updated_at, expires_at, reauth_required, \
                 refresh_retry_mode, refresh_retry_not_before, \
                 (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                  + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) AS backend_now, metadata, NULL AS operation_kind, \
                 NULL AS operation_sentinel, NULL AS operation_expires_at \
                 FROM credentials \
                 WHERE owner_id = ?1 AND record_state = 'live' ORDER BY id",
            )
            .bind(owner.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(read_error)?,
        };
        rows.into_iter()
            .map(CredentialHeadRow::into_stored_head)
            .collect()
    }

    #[tracing::instrument(skip_all, fields(credential.operation = "exists"))]
    async fn exists(
        &self,
        selector: &CredentialSelector,
    ) -> Result<bool, CredentialPersistenceError> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM credentials \
             WHERE id = ?1 AND owner_id = ?2 AND record_state = 'live' LIMIT 1",
        )
        .bind(selector.credential_id().to_string())
        .bind(selector.owner().as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(read_error)?;
        Ok(row.is_some())
    }
}

// ── transaction-scoped mutation helpers ───────────────────────────────────────

impl SqliteCredentialPersistence {
    async fn create_in_transaction(
        transaction: &mut Transaction<'_, Sqlite>,
        selector: &CredentialSelector,
        create: &CredentialCreate,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let existing_owner: Option<(String,)> =
            sqlx::query_as("SELECT owner_id FROM credentials WHERE id = ?1")
                .bind(&credential_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(read_error)?;
        if let Some((existing_owner,)) = existing_owner {
            return if existing_owner == selector.owner().as_str() {
                Err(CredentialPersistenceError::AlreadyExists {
                    key: CredentialAlreadyExistsKey::Id,
                })
            } else {
                Err(CredentialPersistenceError::NotFound)
            };
        }

        if let Some(name) = create.name() {
            let name_exists: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM credentials \
                 WHERE owner_id = ?1 AND name = ?2 AND record_state = 'live' LIMIT 1",
            )
            .bind(selector.owner().as_str())
            .bind(name)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(read_error)?;
            if name_exists.is_some() {
                return Err(CredentialPersistenceError::AlreadyExists {
                    key: CredentialAlreadyExistsKey::Name,
                });
            }
        }

        let now_ms = Utc::now().timestamp_millis();
        let row: CredentialCommitRow = sqlx::query_as(
            "INSERT INTO credentials \
             (id, name, owner_id, credential_key, state_kind, state_version, \
              data, version, material_epoch, created_at, updated_at, expires_at, \
              reauth_required, metadata, record_state, tombstoned_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 1, ?8, ?8, ?9, ?10, ?11, 'live', NULL) \
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(create.name())
        .bind(selector.owner().as_str())
        .bind(create.credential_key())
        .bind(create.state_kind())
        .bind(i64::from(create.state_version()))
        .bind(create.data().as_ref())
        .bind(now_ms)
        .bind(create.expires_at().map(|value| value.timestamp_millis()))
        .bind(i64::from(create.reauth_required()))
        .bind(meta_to_json(create.metadata())?)
        .fetch_one(&mut **transaction)
        .await
        .map_err(read_error)?;

        row.into_commit()
    }

    async fn replace_in_transaction(
        transaction: &mut Transaction<'_, Sqlite>,
        selector: &CredentialSelector,
        replacement: &CredentialReplacement,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let lifecycle: Option<CredentialLifecycleRow> = sqlx::query_as(
            "SELECT version, material_epoch, credential_key, record_state, reauth_required FROM credentials \
             WHERE id = ?1 AND owner_id = ?2",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(read_error)?;
        let Some(lifecycle) = lifecycle else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if lifecycle.record_state != "live" {
            return if lifecycle.record_state == "tombstoned" {
                Err(CredentialPersistenceError::NotFound)
            } else {
                Err(CredentialPersistenceError::CorruptRecord)
            };
        }
        let current_reauth = match lifecycle.reauth_required {
            0 => false,
            1 => true,
            _ => return Err(CredentialPersistenceError::CorruptRecord),
        };
        if replacement.material_transition().advances_epoch()
            || current_reauth != replacement.reauth_required()
        {
            let blocked: Option<(String,)> = sqlx::query_as(
                "SELECT operation_kind FROM credential_refresh_claims \
                 WHERE owner_id = ?1 AND credential_id = ?2 AND operation_kind = 'revoke'",
            )
            .bind(selector.owner().as_str())
            .bind(&credential_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(read_error)?;
            if blocked.is_some() {
                return Err(CredentialPersistenceError::OperationBlocked {
                    operation: CredentialOperationKind::Revoke,
                });
            }
        }
        let actual_version = stored_version(lifecycle.version)?;
        if actual_version != replacement.expected_version() {
            return Err(CredentialPersistenceError::VersionConflict {
                expected: replacement.expected_version(),
                actual: actual_version,
            });
        }
        let actual_material_epoch = stored_material_epoch(lifecycle.material_epoch)?;
        if let Some(fence) = replacement.fence() {
            if actual_material_epoch != fence.expected_material_epoch() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected: replacement.expected_version(),
                    actual: actual_version,
                });
            }
            if lifecycle.credential_key != fence.expected_credential_key() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected: replacement.expected_version(),
                    actual: actual_version,
                });
            }
        }
        let next_version = actual_version.next_live()?;
        let next_material_epoch = if replacement.material_transition().advances_epoch() {
            actual_material_epoch.next()?
        } else {
            actual_material_epoch
        };

        if let Some(name) = replacement.name() {
            let name_exists: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM credentials \
                 WHERE owner_id = ?1 AND name = ?2 AND id <> ?3 \
                   AND record_state = 'live' LIMIT 1",
            )
            .bind(selector.owner().as_str())
            .bind(name)
            .bind(&credential_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(read_error)?;
            if name_exists.is_some() {
                return Err(CredentialPersistenceError::AlreadyExists {
                    key: CredentialAlreadyExistsKey::Name,
                });
            }
        }

        let now_ms = Utc::now().timestamp_millis();
        let retry_transition =
            retry_gate::encode_material_transition(replacement.material_transition())?;
        let row: Option<CredentialCommitRow> = sqlx::query_as(
            "UPDATE credentials SET \
               name            = ?3, \
               data            = ?4, \
               state_kind      = ?5, \
               state_version   = ?6, \
               version         = ?7, \
               material_epoch  = ?8, \
               updated_at      = ?9, \
               expires_at      = ?10, \
               reauth_required = ?11, \
               metadata        = ?12, \
               refresh_retry_mode = CASE ?13 \
                   WHEN 0 THEN refresh_retry_mode \
                   WHEN 1 THEN NULL \
                   WHEN 2 THEN 'never' \
                   WHEN 3 THEN 'not_before' \
               END, \
               refresh_retry_not_before = CASE ?13 \
                   WHEN 0 THEN refresh_retry_not_before \
                   WHEN 3 THEN (CAST(strftime('%s', 'now') AS INTEGER) * 1000 \
                       + CAST(substr(strftime('%f', 'now'), 4, 3) AS INTEGER)) \
                       + (?14 * 1000) \
                   ELSE NULL \
               END, \
               refresh_retry_phase = CASE ?13 \
                   WHEN 0 THEN refresh_retry_phase \
                   WHEN 1 THEN NULL \
                   ELSE ?15 \
               END, \
               refresh_retry_kind = CASE ?13 \
                   WHEN 0 THEN refresh_retry_kind \
                   WHEN 1 THEN NULL \
                   ELSE ?16 \
               END, \
               refresh_retry_diagnostic_code = CASE ?13 \
                   WHEN 0 THEN refresh_retry_diagnostic_code \
                   WHEN 1 THEN NULL \
                   ELSE ?17 \
               END \
             WHERE id = ?1 AND owner_id = ?2 \
               AND record_state = 'live' AND version = ?18 \
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .bind(replacement.name())
        .bind(replacement.data().as_ref())
        .bind(replacement.state_kind())
        .bind(i64::from(replacement.state_version()))
        .bind(next_version.get())
        .bind(next_material_epoch.get())
        .bind(now_ms)
        .bind(
            replacement
                .expires_at()
                .map(|value| value.timestamp_millis()),
        )
        .bind(i64::from(replacement.reauth_required()))
        .bind(meta_to_json(replacement.metadata())?)
        .bind(retry_transition.code)
        .bind(retry_transition.delay_seconds)
        .bind(retry_transition.phase)
        .bind(retry_transition.kind)
        .bind(retry_transition.diagnostic_code)
        .bind(replacement.expected_version().get())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(read_error)?;

        row.ok_or(CredentialPersistenceError::CorruptRecord)?
            .into_commit()
    }

    async fn tombstone_in_transaction(
        transaction: &mut Transaction<'_, Sqlite>,
        selector: &CredentialSelector,
        tombstone: CredentialTombstone,
    ) -> Result<CredentialCommit, CredentialPersistenceError> {
        let credential_id = selector.credential_id().to_string();
        let lifecycle: Option<CredentialLifecycleRow> = sqlx::query_as(
            "SELECT version, material_epoch, credential_key, record_state, reauth_required FROM credentials \
             WHERE id = ?1 AND owner_id = ?2",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(read_error)?;
        let Some(lifecycle) = lifecycle else {
            return Err(CredentialPersistenceError::NotFound);
        };
        if lifecycle.record_state != "live" {
            return if lifecycle.record_state == "tombstoned" {
                Err(CredentialPersistenceError::NotFound)
            } else {
                Err(CredentialPersistenceError::CorruptRecord)
            };
        }
        let actual_version = stored_version(lifecycle.version)?;
        if actual_version != tombstone.expected_version() {
            return Err(CredentialPersistenceError::VersionConflict {
                expected: tombstone.expected_version(),
                actual: actual_version,
            });
        }
        let next_version = actual_version.next_tombstone()?;
        let now_ms = Utc::now().timestamp_millis();

        let row: Option<CredentialCommitRow> = sqlx::query_as(
            "UPDATE credentials SET \
               name            = NULL, \
               data            = zeroblob(0), \
               version         = ?3, \
               updated_at      = ?4, \
               expires_at      = NULL, \
               reauth_required = 0, \
               metadata        = '{}', \
               record_state    = 'tombstoned', \
               tombstoned_at   = ?4, \
               refresh_retry_mode = NULL, \
               refresh_retry_not_before = NULL, \
               refresh_retry_phase = NULL, \
               refresh_retry_kind = NULL, \
               refresh_retry_diagnostic_code = NULL \
             WHERE id = ?1 AND owner_id = ?2 \
               AND record_state = 'live' AND version = ?5 \
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .bind(next_version.get())
        .bind(now_ms)
        .bind(tombstone.expected_version().get())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(read_error)?;

        row.ok_or(CredentialPersistenceError::CorruptRecord)?
            .into_commit()
    }
}
