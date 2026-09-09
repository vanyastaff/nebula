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
fn record_setup_result<T, E>(result: &Result<T, E>)
where
    E: SchemaSetupFailure,
{
    let span = Span::current();
    match result {
        Ok(_) => {
            span.record("outcome", "ready");
            span.record("error_code", "none");
        },
        Err(error) => record_setup_failure(error),
    }
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn record_setup_failure(error: &impl SchemaSetupFailure) {
    let span = Span::current();
    match error.failure_kind() {
        SetupFailureKind::Rejected => {
            span.record("outcome", "rejected");
            span.record("error_code", "unsupported_schema");
        },
        SetupFailureKind::Unavailable => {
            span.record("outcome", "failed");
            span.record("error_code", "unavailable");
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

#[cfg(all(test, feature = "sqlite"))]
pub(crate) fn assert_sqlite_memory_setup_locked() {
    std::assert_matches!(
        SQLITE_MEMORY_SETUP.try_acquire(),
        Err(tokio::sync::TryAcquireError::NoPermits)
    );
}

#[cfg(all(test, feature = "sqlite"))]
pub(crate) fn assert_sqlite_file_setup_locked(path: &std::path::Path) {
    let base = path.as_os_str().to_string_lossy();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("{base}{SQLITE_SETUP_LOCK_SUFFIX}"))
        .expect("setup sidecar exists after terminal admission");
    std::assert_matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock));
}

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
    acquire_sqlite_memory_setup_guard_until(tokio::time::Instant::now() + SETUP_LOCK_TIMEOUT).await
}

#[cfg(feature = "sqlite")]
async fn acquire_sqlite_memory_setup_guard_until(
    deadline: tokio::time::Instant,
) -> Result<SqliteMemorySetupPermit, CatalogSetupError> {
    if tokio::time::Instant::now() >= deadline {
        record_sqlite_pre_terminal_budget_exhaustion("memory_setup_lock");
        return Err(CatalogSetupError::Unavailable);
    }
    let permit = tokio::time::timeout_at(deadline, SQLITE_MEMORY_SETUP.acquire())
        .await
        .map_err(|_| {
            record_sqlite_pre_terminal_budget_exhaustion("memory_setup_lock");
            CatalogSetupError::Unavailable
        })?
        .map_err(|_| CatalogSetupError::Unavailable)?;
    Ok(SqliteMemorySetupPermit { _permit: permit })
}

/// Suffix of the dedicated mutual-exclusion file for file-backed setup.
///
/// Deliberately not one of SQLite's own sidecar suffixes so that the sidecar
/// probe below cannot observe this file.
#[cfg(feature = "sqlite")]
const SQLITE_SETUP_LOCK_SUFFIX: &str = "-setup-lock";

/// How long schema setup may wait before entering its terminal section.
///
/// For SQLite this is one pre-terminal admission budget shared by connection
/// acquisition, database discovery, setup-lock admission, and any observer
/// connection required for shared-memory verification. Once admitted, SQLite
/// DDL and its postflight checks run to completion because SQLx's blocking
/// SQLite worker cannot be safely cancelled. An in-flight pool acquisition is
/// likewise allowed to settle under `PoolOptions::acquire_timeout` and is
/// rejected if this admission cutoff passed meanwhile. PostgreSQL uses the
/// same duration for advisory-lock acquisition; its existing setup contract is
/// otherwise unchanged.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
const SETUP_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(5);

/// Poll interval while waiting for the file-backed setup lock.
#[cfg(feature = "sqlite")]
const SQLITE_SETUP_LOCK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);

#[cfg(feature = "sqlite")]
pub(crate) async fn acquire_sqlite_file_setup_guard(
    path: std::path::PathBuf,
) -> Result<SqliteFileSetupGuard, CatalogSetupError> {
    acquire_sqlite_file_setup_guard_until(path, tokio::time::Instant::now() + SETUP_LOCK_TIMEOUT)
        .await
}

#[cfg(feature = "sqlite")]
async fn acquire_sqlite_file_setup_guard_until(
    path: std::path::PathBuf,
    deadline: tokio::time::Instant,
) -> Result<SqliteFileSetupGuard, CatalogSetupError> {
    if tokio::time::Instant::now() >= deadline {
        record_sqlite_pre_terminal_budget_exhaustion("file_setup_lock");
        return Err(CatalogSetupError::Unavailable);
    }
    let remaining_budget = deadline.saturating_duration_since(tokio::time::Instant::now());
    let blocking_deadline = std::time::Instant::now() + remaining_budget;
    let lock_attempt = tokio::task::spawn_blocking(move || {
        acquire_sqlite_file_setup_guard_blocking(path, blocking_deadline)
    });
    tokio::time::timeout_at(deadline, lock_attempt)
        .await
        .map_err(|_| {
            record_sqlite_pre_terminal_budget_exhaustion("file_setup_lock");
            CatalogSetupError::Unavailable
        })?
        .map_err(|_| CatalogSetupError::Unavailable)?
}

#[cfg(feature = "sqlite")]
fn acquire_sqlite_file_setup_guard_blocking(
    path: std::path::PathBuf,
    deadline: std::time::Instant,
) -> Result<SqliteFileSetupGuard, CatalogSetupError> {
    use std::fs::{OpenOptions, TryLockError};

    if std::time::Instant::now() >= deadline {
        return Err(CatalogSetupError::Unavailable);
    }
    let base = path.as_os_str().to_string_lossy().into_owned();
    // Lock a dedicated sidecar, never the database file itself. POSIX removes
    // every `fcntl` record lock a process holds on an inode as soon as that
    // process closes any descriptor for it. A separate inode keeps SQLite's
    // lock domain disjoint from schema-setup serialization.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(format!("{base}{SQLITE_SETUP_LOCK_SUFFIX}"))
        .map_err(|_| CatalogSetupError::Unavailable)?;
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(CatalogSetupError::Unavailable);
        }
        match file.try_lock() {
            Ok(()) => {
                // Measure the database under the lock without opening it. A
                // missing database is the empty state of a new catalog.
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
                std::thread::sleep(
                    SQLITE_SETUP_LOCK_POLL_INTERVAL
                        .min(deadline.saturating_duration_since(std::time::Instant::now())),
                );
            },
            Err(TryLockError::Error(_)) => return Err(CatalogSetupError::Unavailable),
        }
    }
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
/// Run policy-specific admission, migration, and postflight on one session.
/// The caller must run this inside an owned terminal section that retains both
/// the setup guard and session until completion; dropping this borrowed future
/// does not cancel work already submitted to SQLite's worker.
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
trait SqliteSetupProbe {
    fn before_connection_acquisition(&self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }

    fn before_first_attempt(&self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }

    fn observe_attempt(&self, _attempt: &ClassifiedAttempt<SqliteDatabaseRows>) {}

    fn before_retry_wait(&self, _attempts: u32) {}
}

#[cfg(feature = "sqlite")]
struct UnobservedSqliteDatabaseDiscovery;

#[cfg(feature = "sqlite")]
impl SqliteSetupProbe for UnobservedSqliteDatabaseDiscovery {}

#[cfg(feature = "sqlite")]
#[derive(Clone, Copy)]
struct SqlitePreTerminalRetryPolicy {
    admission_timeout: std::time::Duration,
    poll_interval: std::time::Duration,
}

#[cfg(feature = "sqlite")]
const SQLITE_PRE_TERMINAL_RETRY_POLICY: SqlitePreTerminalRetryPolicy =
    SqlitePreTerminalRetryPolicy {
        admission_timeout: SETUP_LOCK_TIMEOUT,
        poll_interval: SQLITE_SETUP_LOCK_POLL_INTERVAL,
    };

#[cfg(feature = "sqlite")]
type SqliteDatabaseRows = Vec<(i64, String, String)>;

#[cfg(feature = "sqlite")]
fn record_sqlite_pre_terminal_budget_exhaustion(stage: &'static str) {
    tracing::warn!(
        target: "nebula_storage::migration",
        stage,
        outcome = "budget_exhausted",
        "SQLite schema setup exhausted its pre-terminal admission budget"
    );
}

#[cfg(feature = "sqlite")]
enum ClassifiedAttempt<T> {
    Success(T),
    Transient { numeric_code: Option<u32> },
    NonRetryable { numeric_code: Option<u32> },
}

#[cfg(feature = "sqlite")]
trait SqliteDatabaseDiscoveryAttempt {
    fn run(&mut self) -> impl Future<Output = ClassifiedAttempt<SqliteDatabaseRows>> + Send;
}

#[cfg(feature = "sqlite")]
fn record_sqlite_database_discovery_exhaustion(last_lock_code: Option<u32>, attempts: u32) {
    tracing::warn!(
        target: "nebula_storage::migration",
        stage = "main_database_discovery",
        sqlite_code = ?last_lock_code,
        attempts,
        outcome = "retry_exhausted",
        "SQLite schema setup exhausted its remaining budget during database discovery"
    );
}

#[cfg(feature = "sqlite")]
struct SqliteDatabaseListAttempt<'connection> {
    connection: &'connection mut sqlx::SqliteConnection,
}

#[cfg(feature = "sqlite")]
impl SqliteDatabaseDiscoveryAttempt for SqliteDatabaseListAttempt<'_> {
    async fn run(&mut self) -> ClassifiedAttempt<SqliteDatabaseRows> {
        match sqlx::query_as("PRAGMA database_list")
            .fetch_all(&mut *self.connection)
            .await
        {
            Ok(databases) => ClassifiedAttempt::Success(databases),
            Err(error) if is_transient_sqlite_lock(&error) => ClassifiedAttempt::Transient {
                numeric_code: sqlite_numeric_error_code(&error),
            },
            Err(error) => ClassifiedAttempt::NonRetryable {
                numeric_code: sqlite_numeric_error_code(&error),
            },
        }
    }
}

#[cfg(all(test, feature = "sqlite"))]
async fn retry_sqlite_database_discovery<Attempt, Probe>(
    attempt: &mut Attempt,
    probe: &Probe,
    retry: SqlitePreTerminalRetryPolicy,
) -> Result<SqliteDatabaseRows, CatalogSetupError>
where
    Attempt: SqliteDatabaseDiscoveryAttempt,
    Probe: SqliteSetupProbe,
{
    let deadline = tokio::time::Instant::now() + retry.admission_timeout;
    retry_sqlite_database_discovery_until(attempt, probe, deadline, retry.poll_interval).await
}

#[cfg(feature = "sqlite")]
async fn retry_sqlite_database_discovery_until<Attempt, Probe>(
    attempt: &mut Attempt,
    probe: &Probe,
    deadline: tokio::time::Instant,
    poll_interval: std::time::Duration,
) -> Result<SqliteDatabaseRows, CatalogSetupError>
where
    Attempt: SqliteDatabaseDiscoveryAttempt,
    Probe: SqliteSetupProbe,
{
    let mut attempts = 0_u32;
    let mut last_lock_code = None::<u32>;
    loop {
        if tokio::time::Instant::now() >= deadline {
            record_sqlite_database_discovery_exhaustion(last_lock_code, attempts);
            return Err(CatalogSetupError::Unavailable);
        }
        attempts = attempts.saturating_add(1);
        let attempt_result = tokio::time::timeout_at(deadline, attempt.run()).await;
        let Ok(attempt_result) = attempt_result else {
            record_sqlite_database_discovery_exhaustion(last_lock_code, attempts);
            return Err(CatalogSetupError::Unavailable);
        };
        probe.observe_attempt(&attempt_result);
        match attempt_result {
            ClassifiedAttempt::Success(databases) => {
                if attempts > 1 {
                    tracing::info!(
                        target: "nebula_storage::migration",
                        stage = "main_database_discovery",
                        sqlite_code = ?last_lock_code,
                        attempts,
                        outcome = "recovered",
                        "SQLite schema setup recovered database discovery after a transient lock"
                    );
                }
                return Ok(databases);
            },
            ClassifiedAttempt::Transient { numeric_code } => {
                last_lock_code = numeric_code;
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    record_sqlite_database_discovery_exhaustion(last_lock_code, attempts);
                    return Err(CatalogSetupError::Unavailable);
                }
                let retry_wait = tokio::time::sleep(poll_interval.min(deadline - now));
                tokio::pin!(retry_wait);
                probe.before_retry_wait(attempts);
                retry_wait.await;
            },
            ClassifiedAttempt::NonRetryable { numeric_code } => {
                tracing::warn!(
                    target: "nebula_storage::migration",
                    stage = "main_database_discovery",
                    sqlite_code = ?numeric_code,
                    attempts,
                    outcome = "non_retryable",
                    "SQLite schema setup database discovery failed"
                );
                return Err(CatalogSetupError::Unavailable);
            },
        }
    }
}

#[cfg(feature = "sqlite")]
async fn sqlite_main_database_path_with_probe<Probe>(
    connection: &mut sqlx::SqliteConnection,
    probe: &Probe,
    deadline: tokio::time::Instant,
    poll_interval: std::time::Duration,
) -> Result<std::path::PathBuf, CatalogSetupError>
where
    Probe: SqliteSetupProbe,
{
    probe.before_first_attempt().await;
    let mut attempt = SqliteDatabaseListAttempt { connection };
    let databases =
        retry_sqlite_database_discovery_until(&mut attempt, probe, deadline, poll_interval).await?;
    let main_path = databases
        .into_iter()
        .find_map(|(_, name, file)| (name == "main").then(|| std::path::PathBuf::from(file)));
    if main_path.is_none() {
        tracing::warn!(
            target: "nebula_storage::migration",
            stage = "main_database_discovery",
            sqlite_code = ?None::<u32>,
            outcome = "missing_main",
            "SQLite schema setup did not discover the main database"
        );
    }
    main_path.ok_or(CatalogSetupError::Unavailable)
}

#[cfg(feature = "sqlite")]
async fn verify_shared_memory_visibility(
    mut observer: sqlx::pool::PoolConnection<sqlx::Sqlite>,
) -> Result<(), CatalogSetupError> {
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

#[cfg(feature = "sqlite")]
async fn run_sqlite_memory_terminal_sequence<
    MigrationConnection,
    ObserverConnection,
    Migration,
    MigrationFuture,
    VerifyVisibility,
    VisibilityFuture,
>(
    migration_connection: MigrationConnection,
    observer_connection: Option<ObserverConnection>,
    migrate: Migration,
    verify_visibility: VerifyVisibility,
) -> Result<(), CatalogSetupError>
where
    Migration: FnOnce(MigrationConnection) -> MigrationFuture,
    MigrationFuture: Future<Output = Result<(), CatalogSetupError>>,
    VerifyVisibility: FnOnce(ObserverConnection) -> VisibilityFuture,
    VisibilityFuture: Future<Output = Result<(), CatalogSetupError>>,
{
    migrate(migration_connection).await?;
    if let Some(observer_connection) = observer_connection {
        verify_visibility(observer_connection).await?;
    }
    Ok(())
}

#[cfg(feature = "sqlite")]
async fn run_sqlite_memory_terminal_operations(
    migration_connection: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    observer_connection: Option<sqlx::pool::PoolConnection<sqlx::Sqlite>>,
) -> Result<(), CatalogSetupError> {
    run_sqlite_memory_terminal_sequence(
        migration_connection,
        observer_connection,
        |mut connection| async move {
            migrate_sqlite_connection::<CatalogOnly>(&mut connection).await
        },
        verify_shared_memory_visibility,
    )
    .await
}

#[cfg(feature = "sqlite")]
fn require_sqlite_terminal_setup_admission(
    deadline: tokio::time::Instant,
) -> Result<(), CatalogSetupError> {
    if tokio::time::Instant::now() < deadline {
        Ok(())
    } else {
        record_sqlite_pre_terminal_budget_exhaustion("terminal_setup_admission");
        Err(CatalogSetupError::Unavailable)
    }
}

#[cfg(feature = "sqlite")]
/// Own the setup guard and operation through terminal completion, independently
/// of cancellation of the task observing this result. The operation must own all
/// database sessions and pool handles needed to finish its work.
pub(crate) async fn complete_sqlite_terminal_section<Guard, T, E>(
    setup_guard: Guard,
    operation: impl Future<Output = Result<T, E>> + Send + 'static,
) -> Result<T, E>
where
    Guard: Send + 'static,
    T: Send + 'static,
    E: From<CatalogSetupError> + SchemaSetupFailure + Send + 'static,
{
    let span = Span::current();
    let terminal_owner = tokio::spawn(
        async move {
            let _setup_guard = setup_guard;
            let result = operation.await;
            record_setup_result(&result);
            result
        }
        .instrument(span),
    );
    supervise_sqlite_setup_task(terminal_owner, SqliteSetupTaskStage::Terminal)
        .await
        .map_err(|_| E::from(CatalogSetupError::Unavailable))?
        .map_err(E::from)?
}

#[cfg(feature = "sqlite")]
#[derive(Clone, Copy)]
enum SqliteSetupTaskStage {
    Terminal,
    ConnectionAcquisition,
}

/// Supervision owns the join independently of the result receiver. Losing the
/// receiver never cancels SQLite work or its failure reporting. Runtime shutdown
/// and process-aborting panics remain outside this task-lifetime guarantee.
#[cfg(feature = "sqlite")]
fn supervise_sqlite_setup_task<T: Send + 'static>(
    owner: tokio::task::JoinHandle<T>,
    stage: SqliteSetupTaskStage,
) -> tokio::sync::oneshot::Receiver<Result<T, CatalogSetupError>> {
    use tracing::instrument::WithSubscriber as _;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(
        async move {
            let result = owner.await.map_err(|join_error| {
                let stage_name = match stage {
                    SqliteSetupTaskStage::Terminal => "terminal_setup_task",
                    SqliteSetupTaskStage::ConnectionAcquisition => {
                        "setup_connection_acquisition_task"
                    },
                };
                tracing::error!(
                    target: "nebula_storage::migration",
                    stage = stage_name,
                    task_id = %join_error.id(),
                    task_cancelled = join_error.is_cancelled(),
                    task_panicked = join_error.is_panic(),
                    outcome = "task_failed",
                    "SQLite schema setup task failed before reporting its result"
                );
                if matches!(stage, SqliteSetupTaskStage::Terminal) {
                    record_setup_failure(&CatalogSetupError::Unavailable);
                }
                CatalogSetupError::Unavailable
            });
            // A cancelled observer cannot retain an undeliverable pooled
            // connection: dropping the failed send's value returns it to its pool.
            drop(sender.send(result));
        }
        .instrument(Span::current())
        .with_current_subscriber(),
    );
    receiver
}

/// Whether a `sqlx` failure is SQLite reporting a lock that clears on its own.
///
/// Shared-cache mode returns `SQLITE_LOCKED_SHAREDCACHE` (262) **immediately**
/// and deliberately does not route it through `busy_timeout`: a busy handler
/// cannot resolve a shared-cache lock without deadlocking, so SQLite hands the
/// condition to the application to retry. Every extended result retains its
/// primary result in the low byte, so primary `SQLITE_BUSY` (5) and
/// `SQLITE_LOCKED` (6), including current and future extended forms, are the
/// complete transient lock class.
#[cfg(feature = "sqlite")]
fn is_transient_sqlite_lock(error: &sqlx::Error) -> bool {
    sqlite_numeric_error_code(error).is_some_and(is_transient_sqlite_numeric_code)
}

#[cfg(feature = "sqlite")]
fn sqlite_numeric_error_code(error: &sqlx::Error) -> Option<u32> {
    let sqlx::Error::Database(database_error) = error else {
        return None;
    };
    database_error
        .code()
        .as_deref()
        .and_then(|code| code.parse().ok())
}

#[cfg(all(test, feature = "sqlite"))]
fn is_transient_sqlite_code(code: &str) -> bool {
    code.parse::<u32>()
        .is_ok_and(is_transient_sqlite_numeric_code)
}

#[cfg(feature = "sqlite")]
fn is_transient_sqlite_numeric_code(extended_code: u32) -> bool {
    matches!(extended_code & 0xff, 5 | 6)
}

/// Acquire a pooled connection for schema setup, waiting out a transient lock.
///
/// Concurrent startup is the ordinary case, not an edge: while one connection
/// runs the migration DDL it holds the schema lock, and any *other* connection
/// opening against the same shared-cache database is refused outright. Failing
/// setup on that would make a second replica's boot depend on losing a race it
/// has no way to avoid, so the loser waits for the lock to clear instead —
/// bounded by the setup caller's pre-terminal admission deadline. An admitted
/// peer may continue beyond that deadline because SQLite DDL cannot be safely
/// cancelled once its blocking worker starts.
#[cfg(feature = "sqlite")]
async fn acquire_setup_connection(
    pool: &sqlx::SqlitePool,
    deadline: tokio::time::Instant,
) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, CatalogSetupError> {
    loop {
        if tokio::time::Instant::now() >= deadline {
            record_sqlite_pre_terminal_budget_exhaustion("setup_connection_acquisition");
            return Err(CatalogSetupError::Unavailable);
        }
        // SQLx 0.9 acquisition is not cancellation-safe. Let an owning task
        // settle under the pool's configured `acquire_timeout`; if this setup
        // caller is cancelled, the detached task returns any acquired
        // connection to the pool instead of abandoning its slot. The absolute
        // deadline remains an admission cutoff, checked again after settling.
        let acquisition_pool = pool.clone();
        let acquisition = settle_sqlite_connection_acquisition_until(
            async move { acquisition_pool.acquire().await },
            deadline,
        )
        .await?;
        if tokio::time::Instant::now() >= deadline {
            drop(acquisition);
            record_sqlite_pre_terminal_budget_exhaustion("setup_connection_acquisition");
            return Err(CatalogSetupError::Unavailable);
        }
        match acquisition {
            Ok(connection) => return Ok(connection),
            Err(error) if is_transient_sqlite_lock(&error) => {
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!(
                        target: "nebula_storage::migration",
                        stage = "setup_connection_acquisition",
                        sqlite_code = ?sqlite_numeric_error_code(&error),
                        outcome = "budget_exhausted",
                        "SQLite schema-setup connection stayed locked past pre-terminal admission"
                    );
                    return Err(CatalogSetupError::Unavailable);
                }
                tokio::time::sleep_until(
                    (tokio::time::Instant::now() + SQLITE_SETUP_LOCK_POLL_INTERVAL).min(deadline),
                )
                .await;
            },
            Err(_) => return Err(CatalogSetupError::Unavailable),
        }
    }
}

#[cfg(feature = "sqlite")]
async fn settle_sqlite_connection_acquisition_until(
    acquisition: impl Future<Output = Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, sqlx::Error>>
    + Send
    + 'static,
    deadline: tokio::time::Instant,
) -> Result<Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, sqlx::Error>, CatalogSetupError> {
    let acquisition_result = supervise_sqlite_setup_task(
        tokio::spawn(acquisition),
        SqliteSetupTaskStage::ConnectionAcquisition,
    );
    let acquisition_result =
        if let Ok(result) = tokio::time::timeout_at(deadline, acquisition_result).await {
            result
        } else {
            // Only the result receiver expires. The supervisor still observes
            // completion under the pool's acquisition timeout and returns any
            // undeliverable connection to the pool.
            record_sqlite_pre_terminal_budget_exhaustion("setup_connection_acquisition");
            return Err(CatalogSetupError::Unavailable);
        };
    acquisition_result.map_err(|_| CatalogSetupError::Unavailable)?
}

#[cfg(feature = "sqlite")]
async fn prepare_sqlite_pool_setup<Connection, Acquire, AcquireFuture, Discover, DiscoverFuture>(
    probe: &impl SqliteSetupProbe,
    retry_policy: SqlitePreTerminalRetryPolicy,
    acquire: Acquire,
    discover: Discover,
) -> Result<(Connection, std::path::PathBuf, tokio::time::Instant), CatalogSetupError>
where
    Acquire: FnOnce(tokio::time::Instant) -> AcquireFuture,
    AcquireFuture: Future<Output = Result<Connection, CatalogSetupError>>,
    Discover: FnOnce(Connection, tokio::time::Instant, std::time::Duration) -> DiscoverFuture,
    DiscoverFuture: Future<Output = Result<(Connection, std::path::PathBuf), CatalogSetupError>>,
{
    let pre_terminal_deadline = tokio::time::Instant::now() + retry_policy.admission_timeout;
    probe.before_connection_acquisition().await;
    let connection = acquire(pre_terminal_deadline).await?;
    let (connection, main_path) = discover(
        connection,
        pre_terminal_deadline,
        retry_policy.poll_interval,
    )
    .await?;
    Ok((connection, main_path, pre_terminal_deadline))
}

#[cfg(feature = "sqlite")]
async fn setup_sqlite_pool_with_discovery_probe<Probe>(
    pool: sqlx::SqlitePool,
    discovery_probe: &Probe,
    retry_policy: SqlitePreTerminalRetryPolicy,
) -> Result<(), CatalogSetupError>
where
    Probe: SqliteSetupProbe,
{
    let span = sqlite_setup_span(<CatalogOnly as AdmissionPolicy<sqlx::SqliteConnection>>::SCOPE);
    async {
        let mut terminal_owner_started = false;
        let result = async {
            // The preparation seam creates one pre-terminal deadline before
            // its first await. Acquisition, discovery, lock admission, and a
            // shared-memory observer all consume that same budget. Once the
            // guarded terminal owner starts, SQLite DDL cannot be cancelled
            // safely; it may outlive both this deadline and its caller while
            // retaining every connection and guard through postflight.
            let (mut connection, main_path, pre_terminal_deadline) = prepare_sqlite_pool_setup(
                discovery_probe,
                retry_policy,
                |deadline| acquire_setup_connection(&pool, deadline),
                |mut connection, deadline, poll_interval| async move {
                    let main_path = sqlite_main_database_path_with_probe(
                        &mut connection,
                        discovery_probe,
                        deadline,
                        poll_interval,
                    )
                    .await?;
                    Ok((connection, main_path))
                },
            )
            .await?;
            let is_memory = main_path.as_os_str().is_empty();
            if is_memory {
                // Do not occupy a pool slot while waiting for the process-wide
                // permit: the winner needs a second slot to prove shared-cache
                // visibility. Once the permit is held, no competing in-process
                // setup can claim one slot and wait for the other. Both owned
                // connections are acquired before terminal admission, so the
                // admitted path never waits on pool capacity.
                drop(connection);
                let setup_guard =
                    acquire_sqlite_memory_setup_guard_until(pre_terminal_deadline).await?;
                let migration_connection =
                    acquire_setup_connection(&pool, pre_terminal_deadline).await?;
                let observer_connection = if pool.options().get_max_connections() > 1 {
                    Some(acquire_setup_connection(&pool, pre_terminal_deadline).await?)
                } else {
                    None
                };
                require_sqlite_terminal_setup_admission(pre_terminal_deadline)?;
                terminal_owner_started = true;
                complete_sqlite_terminal_section(
                    setup_guard,
                    run_sqlite_memory_terminal_operations(
                        migration_connection,
                        observer_connection,
                    ),
                )
                .await?;
            } else {
                let setup_guard =
                    acquire_sqlite_file_setup_guard_until(main_path, pre_terminal_deadline).await?;
                require_sqlite_terminal_setup_admission(pre_terminal_deadline)?;
                terminal_owner_started = true;
                complete_sqlite_terminal_section(setup_guard, async move {
                    migrate_sqlite_connection::<CatalogOnly>(&mut connection).await
                })
                .await?;
            }
            Ok(())
        }
        .await;
        if !terminal_owner_started && let Err(error) = &result {
            record_setup_failure(error);
        }
        result
    }
    .instrument(span)
    .await
}

#[cfg(feature = "sqlite")]
pub(crate) async fn setup_sqlite_pool(pool: sqlx::SqlitePool) -> Result<(), CatalogSetupError> {
    setup_sqlite_pool_with_discovery_probe(
        pool,
        &UnobservedSqliteDatabaseDiscovery,
        SQLITE_PRE_TERMINAL_RETRY_POLICY,
    )
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

#[cfg(all(test, feature = "sqlite"))]
mod sqlite_lock_tests;

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod tests {
    use super::{GENERAL_CATALOG_SUPPORTED_FLOOR, catalog};

    /// Deliberately spelled with literals: this is the tripwire that makes a
    /// new catalog head a decision rather than a side effect. Deriving either
    /// value from the migrator would make it pass automatically and prove
    /// nothing.
    ///
    /// Head 0045 (`port_operation_ledger`) reviewed against the floor: it
    /// creates one new table and touches no existing relation, so it needs no
    /// aggregate-owner validation and the floor stays at 0040. Its `CHECK`
    /// constraints bind only rows the migration itself introduces, so no
    /// database admitted at 0040 or later can hold a row they would reject.
    /// The same review covered 0044 (`control_queue_claim_generation`), which
    /// adds one defaulted column to `port_control_queue` and performs
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
        use super::{is_transient_sqlite_code, is_transient_sqlite_lock};

        // Primary BUSY/LOCKED and their named extended forms: BUSY_RECOVERY,
        // LOCKED_SHAREDCACHE, BUSY_SNAPSHOT, LOCKED_VTAB, BUSY_TIMEOUT.
        for code in ["5", "6", "261", "262", "517", "518", "773"] {
            assert!(
                is_transient_sqlite_code(code),
                "SQLite lock code {code} must enter the bounded retry path"
            );
        }
        for code in ["0", "1", "7", "256", "not-numeric", "-5"] {
            assert!(
                !is_transient_sqlite_code(code),
                "non-lock SQLite code {code} must fail without retry"
            );
        }

        // A non-database error is never a lock.
        assert!(!is_transient_sqlite_lock(&sqlx::Error::PoolClosed));
        assert!(!is_transient_sqlite_lock(&sqlx::Error::WorkerCrashed));
    }

    /// Head 0050 creates accepted-turn markers with the complete set of command
    /// sources used by the runtime owner. It neither infers historical acceptance
    /// nor rewrites aggregate state. Recovery guarantees start with marker-writing
    /// acceptors; deployments must quiesce older acceptors or reconcile their work
    /// through its runtime owner. Head 0049 adds an empty protocol child table and a redundant unique owner
    /// index; it neither upgrades legacy ledger rows nor grants effect authority.
    /// Head 0048 adds an empty immutable bundle table and tenant parent index,
    /// without fabricating contracts for existing executions. Head 0047 adds
    /// nullable activation metadata without rewriting legacy workflow identities.
    /// The preceding 0046 is aggregate-neutral only when
    /// the dispatch queue is empty.
    /// Its SQL preflight rejects every legacy row before any schema change;
    /// successful setup introduces no aggregate mutation or invented identity.
    /// Nonempty deployments must remain at their prior schema until runtime
    /// owners have drained and retired the legacy rows through their own ports.
    /// The rejection is terminal and atomic; it is never classified as a lock.
    #[test]
    fn new_catalog_head_requires_explicit_admission_policy_review() {
        assert_eq!(GENERAL_CATALOG_SUPPORTED_FLOOR, 40);
        #[cfg(feature = "sqlite")]
        assert_eq!(catalog::catalog_head(&super::SQLITE_MIGRATOR), 50);
        #[cfg(feature = "postgres")]
        assert_eq!(catalog::catalog_head(&super::POSTGRES_MIGRATOR), 50);
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
