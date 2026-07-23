//! SQLite-backed `CredentialPersistence` impl.
//!
//! Persists [`StoredCredential`] rows in the structural `credentials` table at
//! migration `0039_credentials_owner_and_record_state.sql`.
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
    CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialOwner,
    CredentialPersistence, CredentialPersistenceError, CredentialReplacement, CredentialSelector,
    CredentialTombstone, CredentialVersion, SecretBytes, StoredCredential, StoredCredentialHead,
    StoredLiveCredential, StoredTombstonedCredential,
};
use serde_json::Value;
use sqlx::{Connection, Sqlite, SqlitePool, Transaction};

#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use super::{
    CredentialStoreStartupError,
    refresh_claim::SqliteRefreshClaimRepo,
    schema::{SQLITE_MIGRATOR, sqlite as schema},
};

static MEMORY_READINESS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
/// The pool must already satisfy the canonical migration-0039 schema.
#[derive(Clone, Debug)]
pub struct SqliteCredentialPersistence {
    pool: SqlitePool,
    commit_dispatcher: SqliteCommitDispatcher,
    #[cfg(test)]
    precommit_fault: Arc<AtomicBool>,
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
    /// `sqlite::memory:` for an ephemeral store. A local file is protected by a
    /// bounded outer file lock while an
    /// immutable read-only preflight, canonical SQLx migration, and postflight
    /// run. The exact `sqlite::memory:` form is serialized process-wide and
    /// uses one physical connection during readiness.
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
            return Self::connect_memory_options(options).await;
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

    async fn connect_memory_options(
        options: sqlx::sqlite::SqliteConnectOptions,
    ) -> Result<Self, CredentialStoreStartupError> {
        let _readiness = MEMORY_READINESS.lock().await;
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
        schema::admit(&mut connection).await?;
        SQLITE_MIGRATOR
            .run(&mut *connection)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        schema::admit(&mut connection).await?;
        drop(connection);

        Ok(Self::from_ready_pool(pool))
    }

    async fn connect_file_options(
        options: sqlx::sqlite::SqliteConnectOptions,
    ) -> Result<Self, CredentialStoreStartupError> {
        #[cfg(test)]
        return Self::connect_file_options_inner(options, None).await;
        #[cfg(not(test))]
        return Self::connect_file_options_inner(options).await;
    }

    #[cfg(test)]
    async fn connect_file_options_with_gate(
        options: sqlx::sqlite::SqliteConnectOptions,
        gate: &ReadinessTestGate,
    ) -> Result<Self, CredentialStoreStartupError> {
        Self::connect_file_options_inner(options, Some(gate)).await
    }

    async fn connect_file_options_inner(
        options: sqlx::sqlite::SqliteConnectOptions,
        #[cfg(test)] gate: Option<&ReadinessTestGate>,
    ) -> Result<Self, CredentialStoreStartupError> {
        let path = options.get_filename().to_owned();
        let lock = acquire_file_lock(&path).await?;
        #[cfg(test)]
        if let Some(gate) = gate {
            gate.lock_acquired.wait().await;
            gate.release.notified().await;
        }
        let metadata = lock
            .metadata()
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;

        if metadata.len() == 0 {
            if sqlite_sidecars_exist(&path) {
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
        SQLITE_MIGRATOR
            .run(&mut *connection)
            .await
            .map_err(|_| CredentialStoreStartupError::Unavailable)?;
        schema::admit(&mut connection).await?;
        drop(connection);
        drop(lock);

        Ok(Self::from_ready_pool(pool))
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
mod tests {
    use std::{str::FromStr, sync::Arc, time::Duration};

    use nebula_storage_port::{
        CredentialOwner, CredentialPersistence, CredentialPersistenceError, CredentialSelector,
        CredentialVersion, StoredCredential,
        store::{ClaimAttempt, RefreshClaimStore, ReplicaId},
    };

    use crate::credential::test_support::{make_credential, make_replacement};

    use super::{ReadinessTestGate, SQLITE_MIGRATOR, SqliteCredentialPersistence, schema};
    use crate::credential::{CredentialSchemaAdmissionReason, CredentialStoreStartupError};

    fn version(value: i64) -> CredentialVersion {
        CredentialVersion::try_from(value).expect("test version must be valid")
    }

    #[tokio::test]
    async fn curated_refresh_claim_repositories_share_the_admitted_private_pool() {
        let store = SqliteCredentialPersistence::connect_memory()
            .await
            .expect("ready in-memory credential store");
        let first = store.refresh_claim_repo();
        let second = store.refresh_claim_repo();
        let credential_id = nebula_core::CredentialId::new();

        let acquired = first
            .try_claim(
                &credential_id,
                &ReplicaId::new("first"),
                Duration::from_secs(30),
            )
            .await
            .expect("first claim attempt");
        assert!(matches!(acquired, ClaimAttempt::Acquired(_)));

        let observed = second
            .try_claim(
                &credential_id,
                &ReplicaId::new("second"),
                Duration::from_secs(30),
            )
            .await
            .expect("second claim attempt");
        assert!(
            matches!(observed, ClaimAttempt::Contended { .. }),
            "separately-created adapters must observe the same durable claim row"
        );
    }

    #[tokio::test]
    async fn post_commit_fault_is_outcome_unknown_without_automatic_retry()
    -> Result<(), CredentialPersistenceError> {
        let store = SqliteCredentialPersistence::connect_memory().await?;
        let owner = CredentialOwner::from_canonical("post-commit-fault-owner");
        let selector = CredentialSelector::new(owner.clone(), nebula_core::CredentialId::new());
        store
            .create(&selector, make_credential(b"version-one"))
            .await?;

        store.arm_post_commit_outcome_unknown();
        let result = store
            .replace(&selector, make_replacement(version(1), b"version-two"))
            .await;
        assert_eq!(result, Err(CredentialPersistenceError::OutcomeUnknown));
        assert_eq!(
            store.injected_post_commit_faults(),
            1,
            "one caller attempt must cross the post-commit fault exactly once"
        );

        let StoredCredential::Live(persisted) = store.get(&selector).await? else {
            panic!("the acknowledged SQL transaction must have persisted a live replacement");
        };
        assert_eq!(persisted.version(), version(2));
        assert_eq!(persisted.data().as_ref(), b"version-two");

        let foreign = CredentialSelector::new(
            CredentialOwner::from_canonical("post-commit-fault-foreign"),
            selector.credential_id(),
        );
        assert_eq!(
            store.get(&foreign).await,
            Err(CredentialPersistenceError::NotFound),
            "the verification read remains owner-qualified"
        );
        Ok(())
    }

    #[tokio::test]
    async fn confirmed_precommit_rollback_is_unavailable_and_preserves_prior_row()
    -> Result<(), CredentialPersistenceError> {
        let store = SqliteCredentialPersistence::connect_memory().await?;
        let selector = CredentialSelector::new(
            CredentialOwner::from_canonical("precommit-rollback-owner"),
            nebula_core::CredentialId::new(),
        );
        store
            .create(&selector, make_credential(b"version-one"))
            .await?;

        store.arm_precommit_failure();
        assert_eq!(
            store
                .replace(&selector, make_replacement(version(1), b"rolled-back"))
                .await,
            Err(CredentialPersistenceError::Unavailable)
        );
        let StoredCredential::Live(persisted) = store.get(&selector).await? else {
            panic!("confirmed rollback must preserve the prior live row");
        };
        assert_eq!(persisted.version(), version(1));
        assert_eq!(persisted.data().as_ref(), b"version-one");
        assert_eq!(store.injected_post_commit_faults(), 0);
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn second_starter_waits_while_first_holds_the_readiness_lock() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let directory = tempfile::tempdir().expect("temporary directory must be created");
                let path = directory.path().join("contended-readiness.sqlite");
                let url = format!("sqlite://{}?mode=rwc", path.display());
                let options = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
                    .expect("temporary SQLite URL must parse")
                    .create_if_missing(true);
                let gate = Arc::new(ReadinessTestGate::new());

                let first_gate = Arc::clone(&gate);
                let first = tokio::task::spawn_local(async move {
                    SqliteCredentialPersistence::connect_file_options_with_gate(
                        options,
                        &first_gate,
                    )
                    .await
                });
                gate.lock_acquired.wait().await;

                let contender_started = Arc::new(tokio::sync::Barrier::new(2));
                let contender_signal = Arc::clone(&contender_started);
                let contender_url = url.clone();
                let mut second = tokio::task::spawn_local(async move {
                    contender_signal.wait().await;
                    SqliteCredentialPersistence::connect(&contender_url).await
                });
                contender_started.wait().await;
                assert!(
                    tokio::time::timeout(Duration::from_millis(75), &mut second)
                        .await
                        .is_err(),
                    "the second starter must remain blocked while the first owns the file lock"
                );

                gate.release.notify_one();
                let first = first
                    .await
                    .expect("first starter task must not panic")
                    .expect("first starter must establish the ready schema");
                let second = second
                    .await
                    .expect("second starter task must not panic")
                    .expect("second starter must observe the ready schema");

                let (head, successful): (i64, i64) = sqlx::query_as(
                    "SELECT MAX(version), COUNT(*) FROM _sqlx_migrations WHERE success = 1",
                )
                .fetch_one(&second.pool)
                .await
                .expect("serialized readiness ledger must be readable");
                assert_eq!(head, 39);
                assert_eq!(
                    successful,
                    i64::try_from(SQLITE_MIGRATOR.iter().count())
                        .expect("migration count must fit in i64")
                );
                drop((first, second));
            })
            .await;
    }

    #[tokio::test]
    async fn rejected_memory_admission_preserves_logical_state() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory rejection fixture must connect");
        sqlx::query("CREATE TABLE unrelated (value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("unledgered relation must seed");
        sqlx::query("INSERT INTO unrelated (value) VALUES ('preserve')")
            .execute(&pool)
            .await
            .expect("unledgered row must seed");

        let schema_before: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, sql FROM sqlite_schema
             WHERE type = 'table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("fixture schema must snapshot");
        let rows_before: Vec<String> =
            sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
                .fetch_all(&pool)
                .await
                .expect("fixture rows must snapshot");

        let mut connection = pool.acquire().await.expect("single memory connection");
        let error = schema::admit(&mut connection)
            .await
            .expect_err("unledgered memory database must fail closed");
        assert!(matches!(
            error,
            CredentialStoreStartupError::UnsupportedSchemaVersion(ref unsupported)
                if unsupported.reason()
                    == &CredentialSchemaAdmissionReason::UnledgeredDatabase
        ));
        drop(connection);

        let schema_after: Vec<(String, String)> = sqlx::query_as(
            "SELECT name, sql FROM sqlite_schema
             WHERE type = 'table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("rejected schema must remain readable");
        let rows_after: Vec<String> =
            sqlx::query_scalar("SELECT value FROM unrelated ORDER BY value")
                .fetch_all(&pool)
                .await
                .expect("rejected rows must remain readable");
        assert_eq!(schema_after, schema_before);
        assert_eq!(rows_after, rows_before);
        pool.close().await;
    }
}

async fn acquire_file_lock(
    path: &std::path::Path,
) -> Result<std::fs::File, CredentialStoreStartupError> {
    use std::fs::{OpenOptions, TryLockError};
    use std::time::Duration;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| CredentialStoreStartupError::Unavailable)?;
    for _ in 0..200 {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            },
            Err(TryLockError::Error(_)) => {
                return Err(CredentialStoreStartupError::Unavailable);
            },
        }
    }
    Err(CredentialStoreStartupError::Unavailable)
}

fn sqlite_sidecars_exist(path: &std::path::Path) -> bool {
    let base = path.as_os_str().to_string_lossy();
    ["-journal", "-wal", "-shm"]
        .iter()
        .any(|suffix| std::path::Path::new(&format!("{base}{suffix}")).exists())
}

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
    created_at: i64,
    updated_at: i64,
    expires_at: Option<i64>,
    reauth_required: i64,
    metadata: String,
    record_state: String,
    tombstoned_at: Option<i64>,
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
        let reauth_required = match self.reauth_required {
            0 => false,
            1 => true,
            _ => return Err(CredentialPersistenceError::CorruptRecord),
        };
        let metadata = json_to_meta(&self.metadata)?;
        validate_name_projection(self.name.as_deref(), &metadata)?;
        StoredCredentialHead::new(
            stored_credential_id(&self.id)?,
            self.name,
            self.credential_key,
            self.state_kind,
            u32::try_from(self.state_version)
                .map_err(|_| CredentialPersistenceError::CorruptRecord)?,
            stored_version(self.version)?,
            millis_to_utc(self.created_at)?,
            millis_to_utc(self.updated_at)?,
            self.expires_at.map(millis_to_utc).transpose()?,
            reauth_required,
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
        let created_at = millis_to_utc(self.created_at)?;
        let updated_at = millis_to_utc(self.updated_at)?;

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
                    created_at,
                    updated_at,
                    self.expires_at.map(millis_to_utc).transpose()?,
                    reauth_required,
                    metadata,
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
    record_state: String,
}

// ── CredentialPersistence impl ──────────────────────────────────────────────────────

#[async_trait]
impl CredentialPersistence for SqliteCredentialPersistence {
    #[tracing::instrument(skip_all, fields(credential.operation = "get"))]
    async fn get(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredential, CredentialPersistenceError> {
        let row: Option<CredentialRow> = sqlx::query_as(
            "SELECT id, name, credential_key, data, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata, \
             record_state, tombstoned_at \
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

    #[tracing::instrument(skip_all, fields(credential.operation = "get_head"))]
    async fn get_head(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
        let row: Option<CredentialHeadRow> = sqlx::query_as(
            "SELECT id, name, credential_key, state_kind, state_version, version, \
             created_at, updated_at, expires_at, reauth_required, metadata \
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
                "SELECT id, name, credential_key, state_kind, state_version, version, \
                 created_at, updated_at, expires_at, reauth_required, metadata \
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
                "SELECT id, name, credential_key, state_kind, state_version, version, \
                 created_at, updated_at, expires_at, reauth_required, metadata \
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
              data, version, created_at, updated_at, expires_at, \
              reauth_required, metadata, record_state, tombstoned_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8, ?9, ?10, ?11, 'live', NULL) \
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
            "SELECT version, record_state FROM credentials \
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
        if actual_version != replacement.expected_version() {
            return Err(CredentialPersistenceError::VersionConflict {
                expected: replacement.expected_version(),
                actual: actual_version,
            });
        }
        let next_version = actual_version.next_live()?;

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
        let row: Option<CredentialCommitRow> = sqlx::query_as(
            "UPDATE credentials SET \
               name            = ?3, \
               data            = ?4, \
               state_kind      = ?5, \
               state_version   = ?6, \
               version         = ?7, \
               updated_at      = ?8, \
               expires_at      = ?9, \
               reauth_required = ?10, \
               metadata        = ?11 \
             WHERE id = ?1 AND owner_id = ?2 \
               AND record_state = 'live' AND version = ?12 \
             RETURNING id, version, record_state, created_at, updated_at, tombstoned_at",
        )
        .bind(&credential_id)
        .bind(selector.owner().as_str())
        .bind(replacement.name())
        .bind(replacement.data().as_ref())
        .bind(replacement.state_kind())
        .bind(i64::from(replacement.state_version()))
        .bind(next_version.get())
        .bind(now_ms)
        .bind(
            replacement
                .expires_at()
                .map(|value| value.timestamp_millis()),
        )
        .bind(i64::from(replacement.reauth_required()))
        .bind(meta_to_json(replacement.metadata())?)
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
            "SELECT version, record_state FROM credentials \
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
               tombstoned_at   = ?4 \
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
