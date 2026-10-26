//! Shared SQLite pool with cancellation-safe connection policy restoration.

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::time::Duration;

use crate::{StorageError, sql_error::storage_error};

/// Read-only connections opened beside the writer of a WAL database.
const READ_CONNECTIONS: u32 = 4;

/// A deployment pool whose release path restores the ordinary busy timeout.
///
/// OAuth admission temporarily requests nonblocking writer acquisition. The
/// pool, rather than the request future, owns restoration so cancellation cannot
/// expose that setting to another borrower. This type does not imply schema or
/// identity-secret admission; those remain explicit startup operations.
///
/// Every write goes through the one writer connection ([`Self::pool`]). A
/// file-backed database in WAL mode also gets a small read-only pool
/// ([`Self::read_pool`]): WAL readers see the last committed snapshot and
/// neither wait for the writer nor block it, so reads routed there no longer
/// queue behind writes on the single connection. Any other database (an
/// in-memory one, or a rollback-journal file) keeps the writer alone.
#[derive(Clone)]
pub struct DeploymentPool {
    pool: SqlitePool,
    readers: Option<SqlitePool>,
}

impl DeploymentPool {
    /// Open one shared, single-connection writer pool with a five-second
    /// lock-wait budget, plus a read-only pool when the database is a file in
    /// WAL mode.
    ///
    /// Writer connections never expire automatically, preserving
    /// single-connection in-memory databases. `options` selects the database
    /// and journal policy; its busy timeout is replaced by the deployment
    /// policy.
    ///
    /// # Errors
    /// Returns a value-free storage error if the database cannot open.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn connect(options: SqliteConnectOptions) -> Result<Self, StorageError> {
        let reader_options = options.clone();
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            // There is no await between taking an idle connection and handing
            // it to its borrower. Cancelling acquisition must not discard an
            // in-memory database during SQLx's otherwise asynchronous ping.
            .test_before_acquire(false)
            .after_release(|connection, _| {
                Box::pin(async move {
                    // SQLx does not return the connection to the idle queue until
                    // this completes. A failed reset discards the connection.
                    sqlx::query("PRAGMA busy_timeout = 5000")
                        .execute(connection)
                        .await?;
                    Ok(true)
                })
            })
            .connect_with(options.busy_timeout(Duration::from_secs(5)))
            .await
            .map_err(storage_error)?;
        // The writer has applied the journal mode; an in-memory database
        // reports `memory` and a rollback-journal file `delete`, and both keep
        // the single connection: a second in-memory connection would open a
        // different, empty database.
        let journal_mode: String = match sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
        {
            Ok(mode) => mode,
            Err(error) => {
                pool.close().await;
                return Err(storage_error(error));
            },
        };
        let readers = if journal_mode.eq_ignore_ascii_case("wal") {
            let readers = SqlitePoolOptions::new()
                .max_connections(READ_CONNECTIONS)
                .connect_with(
                    reader_options
                        .create_if_missing(false)
                        .read_only(true)
                        .busy_timeout(Duration::from_secs(5)),
                )
                .await;
            match readers {
                Ok(readers) => Some(readers),
                Err(error) => {
                    pool.close().await;
                    return Err(storage_error(error));
                },
            }
        } else {
            None
        };
        tracing::debug!(
            read_connections = readers.as_ref().map_or(0, |_| READ_CONNECTIONS),
            "sqlite deployment pool opened"
        );
        Ok(Self { pool, readers })
    }

    /// The writer pool and shutdown authority used by every persistence
    /// adapter. Every write, and every read that must see the transaction it
    /// runs in, goes here.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// The pool for reads outside a write transaction: the read-only pool of
    /// a WAL database, or the writer when there is none. Writes through it
    /// fail with `SQLITE_READONLY`.
    #[must_use]
    pub fn read_pool(&self) -> &SqlitePool {
        self.readers.as_ref().unwrap_or(&self.pool)
    }

    /// Close the read-only pool, then the writer.
    pub async fn close(&self) {
        if let Some(readers) = &self.readers {
            readers.close().await;
        }
        self.pool.close().await;
    }
}

impl std::fmt::Debug for DeploymentPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SqliteDeploymentPool")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nebula_storage_port::Scope;
    use nebula_storage_port::store::ExecutionStore;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};

    use super::DeploymentPool;

    fn wal_file(directory: &tempfile::TempDir) -> SqliteConnectOptions {
        SqliteConnectOptions::new()
            .filename(directory.path().join("deployment.db"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
    }

    #[tokio::test]
    async fn an_in_memory_database_keeps_the_single_writer() {
        let deployment = DeploymentPool::connect(SqliteConnectOptions::new().in_memory(true))
            .await
            .expect("open");
        sqlx::query("CREATE TABLE probe (x INTEGER)")
            .execute(deployment.read_pool())
            .await
            .expect("the read pool of an in-memory database is the writer");
        deployment.close().await;
    }

    #[tokio::test]
    async fn a_wal_file_gets_a_read_only_pool() {
        let directory = tempfile::tempdir().expect("temp dir");
        let deployment = DeploymentPool::connect(wal_file(&directory))
            .await
            .expect("open");
        sqlx::query("CREATE TABLE probe (x INTEGER)")
            .execute(deployment.pool())
            .await
            .expect("the writer writes");
        assert!(
            sqlx::query("INSERT INTO probe VALUES (1)")
                .execute(deployment.read_pool())
                .await
                .is_err(),
            "the read pool must refuse writes"
        );
        deployment.close().await;
    }

    /// A held write transaction occupies the only writer connection. Before
    /// the read pool, an execution read waited for it; now it is answered
    /// from the last committed snapshot.
    #[tokio::test]
    async fn execution_reads_do_not_wait_for_the_writer() {
        let directory = tempfile::tempdir().expect("temp dir");
        let deployment = DeploymentPool::connect(wal_file(&directory))
            .await
            .expect("open");
        crate::sqlite::init_schema(deployment.pool())
            .await
            .expect("schema");
        let store = crate::sqlite::SqliteExecutionStore::new(deployment.pool().clone())
            .with_read_pool(deployment.read_pool().clone());

        let mut writer = deployment.pool().acquire().await.expect("writer");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *writer)
            .await
            .expect("hold the write lock");
        let scope = Scope::new("ws", "org");
        let read = tokio::time::timeout(Duration::from_secs(2), store.get(&scope, "exe_missing"))
            .await
            .expect("the read must not queue behind the held writer");
        assert!(matches!(read, Ok(None)));
        let history = tokio::time::timeout(Duration::from_secs(2), store.count(&scope, None))
            .await
            .expect("the count must not queue behind the held writer");
        assert!(matches!(history, Ok(0)));
        sqlx::query("ROLLBACK")
            .execute(&mut *writer)
            .await
            .expect("release");
        drop(writer);
        deployment.close().await;
    }
}
