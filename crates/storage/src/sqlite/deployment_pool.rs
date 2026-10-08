//! Shared SQLite pool with cancellation-safe connection policy restoration.

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::time::Duration;

use crate::{StorageError, sql_error::storage_error};

/// A deployment pool whose release path restores the ordinary busy timeout.
///
/// OAuth admission temporarily requests nonblocking writer acquisition. The
/// pool, rather than the request future, owns restoration so cancellation cannot
/// expose that setting to another borrower. This type does not imply schema or
/// identity-secret admission; those remain explicit startup operations.
#[derive(Clone)]
pub struct DeploymentPool {
    pool: SqlitePool,
}

impl DeploymentPool {
    /// Open one shared, single-connection pool with a five-second lock-wait budget.
    ///
    /// Connections never expire automatically, preserving single-connection
    /// in-memory databases. `options` selects the database and journal policy;
    /// its busy timeout is replaced by the deployment policy.
    ///
    /// # Errors
    /// Returns a value-free storage error if the database cannot open.
    #[tracing::instrument(level = "debug", skip_all)]
    pub async fn connect(options: SqliteConnectOptions) -> Result<Self, StorageError> {
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
        Ok(Self { pool })
    }

    /// The same pool and shutdown authority used by every persistence adapter.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

impl std::fmt::Debug for DeploymentPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SqliteDeploymentPool")
    }
}
