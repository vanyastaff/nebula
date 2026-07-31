//! Canonical ordered migration catalogs and backend setup coordination.

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use std::future::Future;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use tracing::{Instrument as _, Span};

// Adoption is entirely `sqlx::migrate` ledger manipulation, so it exists only
// where a backend does. Without this gate the module's `use sqlx::migrate::..`
// fails to resolve under `--no-default-features`, which an `--all-features`
// clippy pass cannot see.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) mod adopt;
pub(crate) mod catalog;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use catalog::{CatalogAdmission, CatalogSetupError};

// Prefixes below 0040 require aggregate-owner validation before destructive
// transforms. General schema bootstrap accepts only Fresh or 0040+ catalogs.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const GENERAL_CATALOG_SUPPORTED_FLOOR: i64 = 40;

#[cfg(feature = "sqlite")]
pub(crate) static SQLITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

#[cfg(feature = "postgres")]
pub(crate) static POSTGRES_MIGRATOR: sqlx::migrate::Migrator =
    sqlx::migrate!("./migrations/postgres");

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn unlocked_migrator(canonical: &sqlx::migrate::Migrator) -> sqlx::migrate::Migrator {
    let mut migrator =
        sqlx::migrate::Migrator::with_migrations(canonical.iter().cloned().collect());
    migrator.set_locking(false);
    migrator
}

#[cfg(feature = "sqlite")]
pub(crate) fn unlocked_sqlite_migrator() -> sqlx::migrate::Migrator {
    unlocked_migrator(&SQLITE_MIGRATOR)
}

#[cfg(feature = "postgres")]
pub(crate) fn unlocked_postgres_migrator() -> sqlx::migrate::Migrator {
    unlocked_migrator(&POSTGRES_MIGRATOR)
}

#[derive(Clone, Copy)]
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) enum SetupFailureKind {
    Rejected,
    Unavailable,
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) trait SchemaSetupFailure {
    fn failure_kind(&self) -> SetupFailureKind;
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
impl SchemaSetupFailure for CatalogSetupError {
    fn failure_kind(&self) -> SetupFailureKind {
        match self {
            Self::Rejected(_) => SetupFailureKind::Rejected,
            Self::Unavailable => SetupFailureKind::Unavailable,
        }
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) trait AdmissionPolicy<Connection> {
    type Error: From<CatalogSetupError> + SchemaSetupFailure + Send + Sync + 'static;

    const SCOPE: &'static str;

    fn admit(
        connection: &mut Connection,
    ) -> impl Future<Output = Result<CatalogAdmission, Self::Error>> + Send + '_;
}

#[derive(Clone, Copy)]
#[cfg(any(feature = "sqlite", feature = "postgres"))]
struct CatalogOnly;

#[cfg(feature = "sqlite")]
impl AdmissionPolicy<sqlx::SqliteConnection> for CatalogOnly {
    type Error = CatalogSetupError;

    const SCOPE: &'static str = "catalog";

    fn admit(
        connection: &mut sqlx::SqliteConnection,
    ) -> impl Future<Output = Result<CatalogAdmission, Self::Error>> + Send + '_ {
        catalog::admit_sqlite(connection, GENERAL_CATALOG_SUPPORTED_FLOOR)
    }
}

#[cfg(feature = "postgres")]
impl AdmissionPolicy<sqlx::PgConnection> for CatalogOnly {
    type Error = CatalogSetupError;

    const SCOPE: &'static str = "catalog";

    fn admit(
        connection: &mut sqlx::PgConnection,
    ) -> impl Future<Output = Result<CatalogAdmission, Self::Error>> + Send + '_ {
        catalog::admit_postgres(connection, GENERAL_CATALOG_SUPPORTED_FLOOR)
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn record_admission(admission: CatalogAdmission) {
    let span = Span::current();
    match admission {
        CatalogAdmission::Fresh => {
            span.record("observed_ledger_state", "absent");
            span.record("observed_head", 0_i64);
        },
        CatalogAdmission::CanonicalPrefix { latest } => {
            span.record("observed_ledger_state", "canonical");
            span.record("observed_head", latest);
        },
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn record_setup_result<E>(result: &Result<(), E>)
where
    E: SchemaSetupFailure,
{
    let span = Span::current();
    match result {
        Ok(()) => {
            span.record("outcome", "ready");
            span.record("error_code", "none");
        },
        Err(error) => match error.failure_kind() {
            SetupFailureKind::Rejected => {
                span.record("outcome", "rejected");
                span.record("error_code", "unsupported_schema");
            },
            SetupFailureKind::Unavailable => {
                span.record("outcome", "failed");
                span.record("error_code", "unavailable");
            },
        },
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn require_current_head<E>(admission: CatalogAdmission, expected_head: i64) -> Result<(), E>
where
    E: From<CatalogSetupError>,
{
    match admission {
        CatalogAdmission::CanonicalPrefix { latest } if latest == expected_head => Ok(()),
        CatalogAdmission::Fresh | CatalogAdmission::CanonicalPrefix { .. } => {
            Err(E::from(CatalogSetupError::Unavailable))
        },
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn storage_setup_error(error: CatalogSetupError) -> nebula_storage_port::StorageError {
    match error {
        CatalogSetupError::Rejected(_) => nebula_storage_port::StorageError::Configuration(
            "database schema is not a supported canonical migration prefix".to_owned(),
        ),
        CatalogSetupError::Unavailable => nebula_storage_port::StorageError::Connection(
            "database schema setup unavailable".to_owned(),
        ),
    }
}

#[cfg(feature = "sqlite")]
static SQLITE_MEMORY_SETUP: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[cfg(feature = "sqlite")]
#[must_use = "dropping the permit allows another in-memory schema setup to start"]
pub(crate) struct SqliteMemorySetupPermit {
    _permit: tokio::sync::SemaphorePermit<'static>,
}

#[cfg(feature = "sqlite")]
#[must_use = "dropping the guard allows another file schema setup to start"]
pub(crate) struct SqliteFileSetupGuard {
    _file: std::fs::File,
    initial_len: u64,
    has_sidecar: bool,
}

#[cfg(feature = "sqlite")]
impl SqliteFileSetupGuard {
    pub(crate) fn initial_file_state(&self) -> (u64, bool) {
        (self.initial_len, self.has_sidecar)
    }
}

#[cfg(feature = "sqlite")]
pub(crate) async fn acquire_sqlite_memory_setup_guard()
-> Result<SqliteMemorySetupPermit, CatalogSetupError> {
    let permit = SQLITE_MEMORY_SETUP
        .acquire()
        .await
        .map_err(|_| CatalogSetupError::Unavailable)?;
    Ok(SqliteMemorySetupPermit { _permit: permit })
}

/// Suffix of the dedicated mutual-exclusion file for file-backed setup.
///
/// Deliberately not one of SQLite's own sidecar suffixes so that the sidecar
/// probe below cannot observe this file.
#[cfg(feature = "sqlite")]
const SQLITE_SETUP_LOCK_SUFFIX: &str = "-setup-lock";

/// How long to wait for another process to finish its schema setup.
///
/// This budget covers the winner's *entire* run — a cold catalog is 41
/// migrations plus two admission passes — not just a lock handshake, because a
/// loser holds the wait for exactly as long as the winner works. The previous
/// five seconds was routinely shorter than that run, so a simultaneous
/// multi-replica cold start turned every process but one into a crash-loop:
/// each got `Unavailable`, exited, restarted, and contended again. It stays
/// bounded so a genuinely stuck peer still surfaces rather than hanging.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const SETUP_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);

/// Poll interval while waiting for the file-backed setup lock.
#[cfg(feature = "sqlite")]
const SQLITE_SETUP_LOCK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);

#[cfg(feature = "sqlite")]
pub(crate) async fn acquire_sqlite_file_setup_guard(
    path: std::path::PathBuf,
) -> Result<SqliteFileSetupGuard, CatalogSetupError> {
    use std::fs::{OpenOptions, TryLockError};
    use std::time::Instant;

    tokio::task::spawn_blocking(move || {
        let base = path.as_os_str().to_string_lossy().into_owned();
        // Lock a dedicated sidecar, never the database file itself. POSIX
        // removes *every* `fcntl` record lock a process holds on an inode as
        // soon as that process closes *any* descriptor for it, and SQLite's
        // unix VFS locks with `fcntl`. A second descriptor on the database
        // would therefore silently strip the locks held by live pooled
        // connections in this same process the moment this guard dropped —
        // one of SQLite's documented corruption paths. Opening a different
        // inode keeps the two lock domains disjoint.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(format!("{base}{SQLITE_SETUP_LOCK_SUFFIX}"))
            .map_err(|_| CatalogSetupError::Unavailable)?;
        let deadline = Instant::now() + SETUP_LOCK_TIMEOUT;
        loop {
            match file.try_lock() {
                Ok(()) => {
                    // Measure the database under the lock without opening it.
                    // A missing database reads as empty, exactly as the old
                    // `create(true)` path reported for a freshly created file.
                    let initial_len = match std::fs::metadata(&path) {
                        Ok(metadata) => metadata.len(),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                        Err(_) => return Err(CatalogSetupError::Unavailable),
                    };
                    let has_sidecar = ["-journal", "-wal", "-shm"]
                        .iter()
                        .any(|suffix| std::path::Path::new(&format!("{base}{suffix}")).exists());
                    return Ok(SqliteFileSetupGuard {
                        _file: file,
                        initial_len,
                        has_sidecar,
                    });
                },
                Err(TryLockError::WouldBlock) => {
                    if Instant::now() >= deadline {
                        return Err(CatalogSetupError::Unavailable);
                    }
                    std::thread::sleep(SQLITE_SETUP_LOCK_POLL_INTERVAL);
                },
                Err(TryLockError::Error(_)) => return Err(CatalogSetupError::Unavailable),
            }
        }
    })
    .await
    .map_err(|_| CatalogSetupError::Unavailable)?
}

#[cfg(feature = "sqlite")]
async fn sqlite_foreign_keys_enabled<E>(connection: &mut sqlx::SqliteConnection) -> Result<(), E>
where
    E: From<CatalogSetupError>,
{
    let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(connection)
        .await
        .map_err(|_| E::from(CatalogSetupError::Unavailable))?;
    if enabled == 1 {
        Ok(())
    } else {
        Err(E::from(CatalogSetupError::Unavailable))
    }
}

#[cfg(feature = "sqlite")]
fn sqlite_setup_span(scope: &'static str) -> Span {
    tracing::info_span!(
        "storage_schema_setup",
        backend = "sqlite",
        admission_scope = scope,
        observed_ledger_state = "unknown",
        observed_head = -1_i64,
        outcome = "pending",
        error_code = "none",
    )
}

#[cfg(feature = "sqlite")]
async fn migrate_sqlite_connection<P>(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), P::Error>
where
    P: AdmissionPolicy<sqlx::SqliteConnection>,
{
    sqlite_foreign_keys_enabled::<P::Error>(connection).await?;
    let admission = P::admit(connection).await?;
    record_admission(admission);
    // Run on this exact guarded session. SQLx 0.9's generic
    // `run(Acquire)` obscures that invariant and fails the enclosing
    // future's Send proof under async-trait callers.
    unlocked_sqlite_migrator()
        .run_direct(None, &mut *connection, false)
        .await
        .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
    let postflight = P::admit(connection).await?;
    require_current_head::<P::Error>(postflight, catalog::catalog_head(&SQLITE_MIGRATOR))?;
    sqlite_foreign_keys_enabled::<P::Error>(connection).await
}

#[cfg(feature = "sqlite")]
pub(crate) async fn setup_sqlite_connection_with<P>(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), P::Error>
where
    P: AdmissionPolicy<sqlx::SqliteConnection>,
{
    let span = sqlite_setup_span(P::SCOPE);
    async {
        let result = migrate_sqlite_connection::<P>(connection).await;
        record_setup_result(&result);
        result
    }
    .instrument(span)
    .await
}

#[cfg(feature = "sqlite")]
async fn sqlite_main_database_path(
    connection: &mut sqlx::SqliteConnection,
) -> Result<std::path::PathBuf, CatalogSetupError> {
    let databases: Vec<(i64, String, String)> = sqlx::query_as("PRAGMA database_list")
        .fetch_all(connection)
        .await
        .map_err(|_| CatalogSetupError::Unavailable)?;
    databases
        .into_iter()
        .find_map(|(_, name, file)| (name == "main").then(|| std::path::PathBuf::from(file)))
        .ok_or(CatalogSetupError::Unavailable)
}

#[cfg(feature = "sqlite")]
async fn verify_shared_memory_visibility(pool: &sqlx::SqlitePool) -> Result<(), CatalogSetupError> {
    use std::time::Duration;

    let mut observer = tokio::time::timeout(Duration::from_secs(5), acquire_setup_connection(pool))
        .await
        .map_err(|_| CatalogSetupError::Unavailable)??;
    sqlite_foreign_keys_enabled::<CatalogSetupError>(&mut observer).await?;
    let observed_head: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&mut *observer)
            .await
            .map_err(|_| CatalogSetupError::Unavailable)?;
    if observed_head == Some(catalog::catalog_head(&SQLITE_MIGRATOR)) {
        let admission =
            <CatalogOnly as AdmissionPolicy<sqlx::SqliteConnection>>::admit(&mut observer).await?;
        require_current_head::<CatalogSetupError>(
            admission,
            catalog::catalog_head(&SQLITE_MIGRATOR),
        )
    } else {
        Err(CatalogSetupError::Unavailable)
    }
}

/// Whether a `sqlx` failure is SQLite reporting a lock that clears on its own.
///
/// Shared-cache mode returns `SQLITE_LOCKED_SHAREDCACHE` (262) **immediately**
/// and deliberately does not route it through `busy_timeout`: a busy handler
/// cannot resolve a shared-cache lock without deadlocking, so SQLite hands the
/// condition to the application to retry. `SQLITE_BUSY` (5) and plain
/// `SQLITE_LOCKED` (6), plus their extended forms, are the same class.
#[cfg(feature = "sqlite")]
fn is_transient_sqlite_lock(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(database_error) = error else {
        return false;
    };
    // Primary codes 5 (BUSY) and 6 (LOCKED), and the extended codes that carry
    // them in the low byte — 261 BUSY_SNAPSHOT, 262 LOCKED_SHAREDCACHE,
    // 517 BUSY_TIMEOUT.
    matches!(
        database_error.code().as_deref(),
        Some("5" | "6" | "261" | "262" | "517")
    )
}

/// Acquire a pooled connection for schema setup, waiting out a transient lock.
///
/// Concurrent startup is the ordinary case, not an edge: while one connection
/// runs the migration DDL it holds the schema lock, and any *other* connection
/// opening against the same shared-cache database is refused outright. Failing
/// setup on that would make a second replica's boot depend on losing a race it
/// has no way to avoid, so the loser waits for the lock to clear instead —
/// bounded by the same budget that covers a peer's whole migration run, so a
/// genuinely stuck peer still surfaces.
#[cfg(feature = "sqlite")]
async fn acquire_setup_connection(
    pool: &sqlx::SqlitePool,
) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, CatalogSetupError> {
    let deadline = tokio::time::Instant::now() + SETUP_LOCK_TIMEOUT;
    loop {
        match pool.acquire().await {
            Ok(connection) => return Ok(connection),
            Err(error) if is_transient_sqlite_lock(&error) => {
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!(
                        target: "nebula_storage::migration",
                        %error,
                        "schema-setup connection stayed locked past the setup budget"
                    );
                    return Err(CatalogSetupError::Unavailable);
                }
                tokio::time::sleep(SQLITE_SETUP_LOCK_POLL_INTERVAL).await;
            },
            Err(_) => return Err(CatalogSetupError::Unavailable),
        }
    }
}

#[cfg(feature = "sqlite")]
pub(crate) async fn setup_sqlite_pool(pool: sqlx::SqlitePool) -> Result<(), CatalogSetupError> {
    let span = sqlite_setup_span(<CatalogOnly as AdmissionPolicy<sqlx::SqliteConnection>>::SCOPE);
    async {
        let result = async {
            let mut connection = acquire_setup_connection(&pool).await?;
            let main_path = sqlite_main_database_path(&mut connection).await?;
            let is_memory = main_path.as_os_str().is_empty();
            if is_memory {
                // Do not occupy a pool slot while waiting for the process-wide
                // permit: the winner needs a second slot to prove shared-cache
                // visibility.
                drop(connection);
                let _setup_guard = acquire_sqlite_memory_setup_guard().await?;
                let mut migration_connection = acquire_setup_connection(&pool).await?;
                migrate_sqlite_connection::<CatalogOnly>(&mut migration_connection).await?;
                if pool.options().get_max_connections() > 1 {
                    verify_shared_memory_visibility(&pool).await?;
                }
            } else {
                let _setup_guard = acquire_sqlite_file_setup_guard(main_path).await?;
                migrate_sqlite_connection::<CatalogOnly>(&mut connection).await?;
            }
            Ok(())
        }
        .await;
        record_setup_result(&result);
        result
    }
    .instrument(span)
    .await
}

/// Adopt an unledgered SQLite database by stamping a canonical ledger.
///
/// Runs inside one transaction and re-admits the stamped ledger before
/// committing, so a database that would still be rejected is left exactly as
/// it was rather than carrying a half-written ledger.
#[cfg(feature = "sqlite")]
pub(crate) async fn adopt_sqlite_ledger(
    pool: &sqlx::SqlitePool,
    through_version: i64,
) -> Result<adopt::LedgerAdoptionOutcome, adopt::LedgerAdoptionError> {
    use adopt::{AdoptionPlan, LedgerAdoptionError};

    let mut connection = pool
        .acquire()
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;

    let observation = catalog::sqlite::observe(&mut connection)
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;
    let through_version =
        match adopt::plan_adoption(&unlocked_sqlite_migrator(), &observation, through_version)? {
            AdoptionPlan::Skip(outcome) => return Ok(outcome),
            AdoptionPlan::Stamp { through_version } => through_version,
        };

    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;

    let stamped = adopt::stamp_ledger(
        &mut *connection,
        &unlocked_sqlite_migrator(),
        through_version,
    )
    .await;
    let verified = match stamped {
        Ok(()) => catalog::sqlite::admit(&mut connection, GENERAL_CATALOG_SUPPORTED_FLOOR)
            .await
            .map(|_| ())
            .map_err(|_| LedgerAdoptionError::RejectedAfterStamp),
        Err(error) => Err(error),
    };

    match verified {
        Ok(()) => {
            sqlx::query("COMMIT")
                .execute(&mut *connection)
                .await
                .map_err(|_| LedgerAdoptionError::Unavailable)?;
            Ok(adopt::LedgerAdoptionOutcome::Adopted { through_version })
        },
        Err(error) => {
            // The caller already has a failure to report; a rollback that
            // itself fails must not mask it.
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        },
    }
}

/// Adopt an unledgered PostgreSQL database by stamping a canonical ledger.
///
/// Same contract as [`adopt_sqlite_ledger`].
#[cfg(feature = "postgres")]
pub(crate) async fn adopt_postgres_ledger(
    pool: &sqlx::PgPool,
    through_version: i64,
) -> Result<adopt::LedgerAdoptionOutcome, adopt::LedgerAdoptionError> {
    use adopt::{AdoptionPlan, LedgerAdoptionError};

    let mut connection = pool
        .acquire()
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;

    let observation = catalog::postgres::observe(&mut connection)
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;
    let through_version =
        match adopt::plan_adoption(&unlocked_postgres_migrator(), &observation, through_version)? {
            AdoptionPlan::Skip(outcome) => return Ok(outcome),
            AdoptionPlan::Stamp { through_version } => through_version,
        };

    sqlx::query("BEGIN")
        .execute(&mut *connection)
        .await
        .map_err(|_| LedgerAdoptionError::Unavailable)?;

    let stamped = adopt::stamp_ledger(
        &mut *connection,
        &unlocked_postgres_migrator(),
        through_version,
    )
    .await;
    let verified = match stamped {
        Ok(()) => catalog::postgres::admit(&mut connection, GENERAL_CATALOG_SUPPORTED_FLOOR)
            .await
            .map(|_| ())
            .map_err(|_| LedgerAdoptionError::RejectedAfterStamp),
        Err(error) => Err(error),
    };

    match verified {
        Ok(()) => {
            sqlx::query("COMMIT")
                .execute(&mut *connection)
                .await
                .map_err(|_| LedgerAdoptionError::Unavailable)?;
            Ok(adopt::LedgerAdoptionOutcome::Adopted { through_version })
        },
        Err(error) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            Err(error)
        },
    }
}

#[cfg(feature = "postgres")]
async fn postgres_lock_key<E>(connection: &mut sqlx::PgConnection) -> Result<i64, E>
where
    E: From<CatalogSetupError>,
{
    // Keep this historical namespace stable while old and new binaries may
    // share one database; changing it would split mutual exclusion.
    sqlx::query_scalar(
        "SELECT hashtextextended(
             'nebula:credential-schema:' || current_database() || ':' || current_schema(),
             0
         )",
    )
    .fetch_one(connection)
    .await
    .map_err(|_| E::from(CatalogSetupError::Unavailable))
}

#[cfg(feature = "postgres")]
async fn postgres_read_only_admission<P>(
    connection: &mut sqlx::PgConnection,
) -> Result<CatalogAdmission, P::Error>
where
    P: AdmissionPolicy<sqlx::PgConnection>,
{
    sqlx::query("BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *connection)
        .await
        .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
    let admission = P::admit(&mut *connection).await;
    let finish = if admission.is_ok() {
        "COMMIT"
    } else {
        "ROLLBACK"
    };
    sqlx::query(finish)
        .execute(&mut *connection)
        .await
        .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
    admission
}

#[cfg(feature = "postgres")]
async fn setup_postgres_connection<P>(connection: &mut sqlx::PgConnection) -> Result<(), P::Error>
where
    P: AdmissionPolicy<sqlx::PgConnection>,
{
    let admission = postgres_read_only_admission::<P>(connection).await?;
    record_admission(admission);
    // Run on the exact session that owns the advisory lock. SQLx 0.9's generic
    // `run(Acquire)` obscures that invariant and triggers its HRTB Send failure.
    unlocked_postgres_migrator()
        .run_direct(None, &mut *connection, false)
        .await
        .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
    let postflight = postgres_read_only_admission::<P>(connection).await?;
    require_current_head::<P::Error>(postflight, catalog::catalog_head(&POSTGRES_MIGRATOR))
}

#[cfg(feature = "postgres")]
pub(crate) async fn setup_postgres_pool(pool: sqlx::PgPool) -> Result<(), CatalogSetupError> {
    setup_postgres_pool_with::<CatalogOnly>(pool).await
}

#[cfg(feature = "postgres")]
pub(crate) async fn setup_postgres_pool_with<P>(pool: sqlx::PgPool) -> Result<(), P::Error>
where
    P: AdmissionPolicy<sqlx::PgConnection>,
{
    use sqlx::postgres::{PgAdvisoryLock, PgAdvisoryLockKey};
    use std::time::Duration;

    // Releasing an already-held lock is a single round trip, so it keeps a
    // short bound; acquiring waits out another replica's whole catalog run.
    const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

    let span = tracing::info_span!(
        "storage_schema_setup",
        backend = "postgres",
        admission_scope = P::SCOPE,
        observed_ledger_state = "unknown",
        observed_head = -1_i64,
        outcome = "pending",
        error_code = "none",
    );
    async {
        let result = async {
            let mut connection = pool
                .acquire()
                .await
                .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
            connection.close_on_drop();
            let lock_key = postgres_lock_key::<P::Error>(&mut connection).await?;
            let lock = PgAdvisoryLock::with_key(PgAdvisoryLockKey::BigInt(lock_key));
            let mut guard = tokio::time::timeout(SETUP_LOCK_TIMEOUT, lock.acquire(connection))
                .await
                .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?
                .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
            setup_postgres_connection::<P>(&mut guard).await?;
            let retired_connection = tokio::time::timeout(RELEASE_TIMEOUT, guard.release_now())
                .await
                .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?
                .map_err(|_| P::Error::from(CatalogSetupError::Unavailable))?;
            drop(retired_connection);
            Ok(())
        }
        .await;
        record_setup_result(&result);
        result
    }
    .instrument(span)
    .await
}

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod tests {
    use super::{GENERAL_CATALOG_SUPPORTED_FLOOR, catalog};

    /// Deliberately spelled with literals: this is the tripwire that makes a
    /// new catalog head a decision rather than a side effect. Deriving either
    /// value from the migrator would make it pass automatically and prove
    /// nothing.
    ///
    /// Head 0044 (`control_queue_claim_generation`) reviewed against the
    /// floor: it adds one defaulted column to `port_control_queue` and performs
    /// no destructive transform, so it needs no aggregate-owner validation and
    /// the floor stays at 0040. The same review covered 0043
    /// (`port_start_key_reservations`), which creates one new table:
    /// it creates one new table and touches no existing relation, so it needs
    /// no aggregate-owner validation and the floor stays at 0040. The same
    /// review covered 0042 (`job_dispatch_claim_generation`), which adds one
    /// defaulted column and performs no destructive transform. A database
    /// admitted at 0040 or later still reaches this head by ordinary forward
    /// migration.
    /// The lock classifier decides whether setup waits or fails.
    ///
    /// Both directions are load-bearing: treating a real failure as transient
    /// would park startup until the whole setup budget elapsed, and treating
    /// `SQLITE_LOCKED_SHAREDCACHE` as fatal is the concurrent-startup flake it
    /// was added to remove.
    #[cfg(feature = "sqlite")]
    #[test]
    fn only_sqlite_lock_codes_are_treated_as_transient() {
        use super::is_transient_sqlite_lock;

        // A non-database error is never a lock.
        assert!(!is_transient_sqlite_lock(&sqlx::Error::PoolClosed));
        assert!(!is_transient_sqlite_lock(&sqlx::Error::WorkerCrashed));
    }

    #[test]
    fn new_catalog_head_requires_explicit_admission_policy_review() {
        assert_eq!(GENERAL_CATALOG_SUPPORTED_FLOOR, 40);
        #[cfg(feature = "sqlite")]
        assert_eq!(catalog::catalog_head(&super::SQLITE_MIGRATOR), 44);
        #[cfg(feature = "postgres")]
        assert_eq!(catalog::catalog_head(&super::POSTGRES_MIGRATOR), 44);
    }

    /// The setup guard must never hold a descriptor on the database file.
    ///
    /// POSIX drops every `fcntl` record lock a process holds on an inode when
    /// that process closes any descriptor for it, and SQLite locks with
    /// `fcntl` — so a guard descriptor on the database would strip the locks
    /// of live pooled connections in this process when it dropped. Observing
    /// that the database is never even created proves the guard opened a
    /// different inode.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn file_setup_guard_never_opens_the_database_file() {
        let directory = tempfile::tempdir().expect("temp dir must be creatable");
        let database = directory.path().join("nebula.db");

        let guard = super::acquire_sqlite_file_setup_guard(database.clone())
            .await
            .expect("guard must be acquirable for a fresh database path");

        assert!(
            !database.exists(),
            "the guard must not materialize the database file; a descriptor on it \
             would release this process's SQLite locks when the guard dropped"
        );
        assert_eq!(
            guard.initial_file_state(),
            (0, false),
            "a missing database must still read as empty with no sidecar"
        );
        assert!(
            directory
                .path()
                .join(format!("nebula.db{}", super::SQLITE_SETUP_LOCK_SUFFIX))
                .exists(),
            "the guard must take its lock on a dedicated sidecar inode"
        );
    }

    /// The lock file is not one of SQLite's sidecars, so it must not be
    /// mistaken for a hot journal by the sidecar probe.
    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn setup_lock_file_is_not_seen_as_a_sqlite_sidecar() {
        let directory = tempfile::tempdir().expect("temp dir must be creatable");
        let database = directory.path().join("nebula.db");
        std::fs::write(&database, b"not empty").expect("database file must be writable");

        let guard = super::acquire_sqlite_file_setup_guard(database.clone())
            .await
            .expect("guard must be acquirable");

        let (initial_len, has_sidecar) = guard.initial_file_state();
        assert_eq!(initial_len, 9, "the database's own length must be reported");
        assert!(
            !has_sidecar,
            "the guard's own lock file must not register as a SQLite sidecar"
        );
    }
}
