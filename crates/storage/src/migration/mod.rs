//! Canonical ordered migration catalogs and backend setup coordination.

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use std::future::Future;

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use tracing::{Instrument as _, Span};

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

#[cfg(feature = "sqlite")]
pub(crate) async fn acquire_sqlite_file_setup_guard(
    path: std::path::PathBuf,
) -> Result<SqliteFileSetupGuard, CatalogSetupError> {
    use std::fs::{OpenOptions, TryLockError};
    use std::time::Duration;

    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| CatalogSetupError::Unavailable)?;
        for _ in 0..200 {
            match file.try_lock() {
                Ok(()) => {
                    let initial_len = file
                        .metadata()
                        .map_err(|_| CatalogSetupError::Unavailable)?
                        .len();
                    let base = path.as_os_str().to_string_lossy();
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
                    std::thread::sleep(Duration::from_millis(25));
                },
                Err(TryLockError::Error(_)) => return Err(CatalogSetupError::Unavailable),
            }
        }
        Err(CatalogSetupError::Unavailable)
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

    let mut observer = tokio::time::timeout(Duration::from_secs(5), pool.acquire())
        .await
        .map_err(|_| CatalogSetupError::Unavailable)?
        .map_err(|_| CatalogSetupError::Unavailable)?;
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
pub(crate) async fn setup_sqlite_pool(pool: sqlx::SqlitePool) -> Result<(), CatalogSetupError> {
    let span = sqlite_setup_span(<CatalogOnly as AdmissionPolicy<sqlx::SqliteConnection>>::SCOPE);
    async {
        let result = async {
            let mut connection = pool
                .acquire()
                .await
                .map_err(|_| CatalogSetupError::Unavailable)?;
            let main_path = sqlite_main_database_path(&mut connection).await?;
            let is_memory = main_path.as_os_str().is_empty();
            if is_memory {
                // Do not occupy a pool slot while waiting for the process-wide
                // permit: the winner needs a second slot to prove shared-cache
                // visibility.
                drop(connection);
                let _setup_guard = acquire_sqlite_memory_setup_guard().await?;
                let mut migration_connection = pool
                    .acquire()
                    .await
                    .map_err(|_| CatalogSetupError::Unavailable)?;
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

    const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
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
            let mut guard = tokio::time::timeout(LOCK_TIMEOUT, lock.acquire(connection))
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

    #[test]
    fn new_catalog_head_requires_explicit_admission_policy_review() {
        assert_eq!(GENERAL_CATALOG_SUPPORTED_FLOOR, 40);
        #[cfg(feature = "sqlite")]
        assert_eq!(catalog::catalog_head(&super::SQLITE_MIGRATOR), 41);
        #[cfg(feature = "postgres")]
        assert_eq!(catalog::catalog_head(&super::POSTGRES_MIGRATOR), 41);
    }
}
