//! Async startup logic for the core-flavor worker binary.
//!
//! Separated from `main.rs` so the shutdown path and store wiring are testable
//! without starting the full `#[tokio::main]` harness. The `run` function is the
//! single entry point called by `main`.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage::sqlite::{
    SqliteExecutionStore, SqliteIdempotencyGuard, SqliteJobDispatchQueue, SqliteJournalReader,
    SqliteResumeTokenStore, SqliteWorkflowStore, SqliteWorkflowVersionStore, init_schema,
};
use nebula_storage::{InMemoryCheckpointStore, InMemoryNodeResultStore};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, fmt};

use nebula_engine::{ExecutionStores, WorkflowStores};
use nebula_storage_port::store::JobDispatchQueue;
use nebula_worker_bin::compose::{
    ComposeError, WorkerConfig, WorkerConfigError, build_core_flavor_runtime,
};

/// Top-level error union for the worker binary startup.
///
/// Each variant carries a user-readable [`Display`](std::fmt::Display) message
/// that explains what failed and what to check. `main` walks the source chain
/// via `std::error::Error::source` to surface nested causes.
#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum WorkerRunError {
    /// Environment config is invalid (bad env var format).
    #[error("configuration error — check NEBULA_WORKER_* env vars: {0}")]
    Config(#[from] WorkerConfigError),

    /// SQLite pool construction or `connect()` failed.
    ///
    /// Uses an explicit `.map_err(WorkerRunError::SqliteDatabase)` at the pool
    /// build site rather than `#[from]` so the Postgres variant can reuse the
    /// same `sqlx::Error` without a type-level conflict.
    #[error(
        "SQLite connection failed — check NEBULA_WORKER_DB_PATH and directory permissions: {0}"
    )]
    SqliteDatabase(sqlx::Error),

    /// Postgres pool construction or `connect()` failed.
    ///
    /// Only compiled when the `postgres` cargo feature is enabled.
    #[cfg(feature = "postgres")]
    #[error(
        "Postgres connection failed — check NEBULA_WORKER_DATABASE_URL and network/TLS settings: {0}"
    )]
    PostgresDatabase(sqlx::Error),

    /// `NEBULA_WORKER_DATABASE_URL` is set but this binary was compiled without
    /// the `postgres` cargo feature.
    ///
    /// Gated to `#[cfg(not(feature = "postgres"))]` so it only EXISTS when its
    /// sole constructor — the `#[cfg(not(feature = "postgres"))]` twin of
    /// `build_pg_stores` — is also compiled in. When `--features postgres` is
    /// enabled both are absent and operators hitting the Postgres path get
    /// `PostgresDatabase` instead.
    #[cfg(not(feature = "postgres"))]
    #[error(
        "NEBULA_WORKER_DATABASE_URL is set but this binary was not compiled with the `postgres` \
         feature — rebuild with `--features postgres` or unset the variable to use SQLite"
    )]
    PostgresFeatureNotEnabled,

    /// Schema DDL (`CREATE TABLE IF NOT EXISTS`) failed.
    #[error("schema init failed — the database file may be corrupt or read-only: {0}")]
    Schema(#[from] nebula_storage_port::StorageError),

    /// Plugin wiring or worker runtime assembly failed.
    #[error("composition failed — this is likely a build or config bug: {0}")]
    Compose(#[from] ComposeError),

    /// `WorkerRuntimeBuilder::build` failed (e.g. empty plugin set).
    #[error("runtime build failed: {0}")]
    Runtime(#[from] nebula_worker::WorkerBuildError),

    /// Signal listener setup failed (OS-level error).
    #[error("signal handler setup failed: {0}")]
    Signal(#[from] std::io::Error),
}

/// Build the durable store bundle for the configured backend.
///
/// When `config.database_url` is `None`, the SQLite path is used (WAL +
/// NORMAL synchronous; single connection; `init_schema` applied). This is the
/// default and the only path exercised by CI integration tests.
///
/// When `config.database_url` is `Some`, the Postgres path is used — but
/// only when the binary is compiled with `--features postgres`. Without that
/// feature the call returns [`WorkerRunError::PostgresFeatureNotEnabled`]
/// immediately, never silently falling back to SQLite.
///
/// The Postgres path is compile-verified (`cargo check --all-features`) but is
/// NOT integration-tested in CI (no DATABASE_URL available in the CI
/// environment). SQLite is the default and the tested path.
async fn build_stores(
    config: &WorkerConfig,
) -> Result<(ExecutionStores, WorkflowStores, Arc<dyn JobDispatchQueue>), WorkerRunError> {
    if config.database_url.is_none() {
        // ── SQLite path (default, CI-tested) ─────────────────────────────────
        //
        // `BEGIN IMMEDIATE` CAS + claim-fencing in the store are only correct
        // when a single connection serialises all writes. WAL + NORMAL
        // synchronous gives read/write concurrency without fsync on every
        // commit; busy_timeout prevents instant SQLITE_BUSY errors if another
        // tool (e.g. a CLI probe) briefly holds the WAL write lock.
        //
        // **This SQLite file is not shareable across processes or hosts.** For
        // multi-worker deployments, configure the Postgres backend and share
        // the connection string via NEBULA_WORKER_DATABASE_URL.
        let opts = SqliteConnectOptions::new()
            .filename(&config.db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(WorkerRunError::SqliteDatabase)?;

        // Apply the port-scoped DDL — idempotent `CREATE TABLE IF NOT EXISTS`.
        init_schema(&pool).await?;

        tracing::info!(db_path = %config.db_path, "SQLite schema applied");

        // Every store clone holds the same `SqlitePool` `Arc`; single
        // max_connections serialises writes at the connection level.
        let execution_store = Arc::new(SqliteExecutionStore::new(pool.clone()));
        let journal_reader = Arc::new(SqliteJournalReader::new(pool.clone()));
        // NodeResultStore and CheckpointStore have no SQLite-backed
        // implementation yet; both store transient in-process data (node
        // output slots and stateful checkpoints) that are written and read
        // within a single execution lifetime. Durability is provided by the
        // ExecutionStore single-JSON-blob state machine, not by these auxiliary
        // stores. On a crash, the reclaim sweep re-delivers the job and the
        // engine re-executes the affected nodes from the last persisted state.
        let node_results = Arc::new(InMemoryNodeResultStore::new());
        let checkpoints = Arc::new(InMemoryCheckpointStore::new());
        tracing::warn!(
            "node-result and checkpoint stores are in-memory (not persisted across restarts); \
             crash-recovery re-executes affected nodes via the reclaim sweep — \
             authoritative execution state is the SQLite execution row"
        );
        let idempotency = Arc::new(SqliteIdempotencyGuard::new(pool.clone()));
        let resume_tokens = Arc::new(SqliteResumeTokenStore::new(pool.clone()));
        let workflow_store = Arc::new(SqliteWorkflowStore::new(pool.clone()));
        let versions_store = Arc::new(SqliteWorkflowVersionStore::new(pool.clone()));
        let queue = Arc::new(SqliteJobDispatchQueue::new(pool));

        let execution_stores = ExecutionStores {
            execution: execution_store,
            journal: journal_reader,
            node_results,
            checkpoints,
            idempotency,
            resume_tokens,
        };
        let workflow_stores = WorkflowStores {
            workflow: workflow_store,
            versions: versions_store,
        };

        return Ok((execution_stores, workflow_stores, queue));
    }

    // ── Postgres path (opt-in; compile-verified but not CI-integration-tested) ──
    //
    // `database_url` is `Some` here — the `is_none()` early-return above means
    // this branch is only reached when a DSN was supplied. Extract it once and
    // pass the `&str` in so `build_pg_stores` never needs to touch `Option`.
    let dsn = config
        .database_url
        .as_deref()
        .unwrap_or_else(|| unreachable!("database_url is Some — checked above"));
    build_pg_stores(dsn).await
}

/// Postgres store assembly — compiled only when `--features postgres` is
/// present, and called only when `NEBULA_WORKER_DATABASE_URL` is set.
///
/// `dsn` is the already-extracted connection string. `build_stores` owns the
/// `Option` unwrap and passes the inner `&str` here, so this function never
/// touches `Option` and carries no panic path.
///
/// The `#[cfg(not(feature = "postgres"))]` twin always returns
/// `WorkerRunError::PostgresFeatureNotEnabled` so the fail-closed invariant
/// holds regardless of which binary was deployed.
#[cfg(feature = "postgres")]
async fn build_pg_stores(
    dsn: &str,
) -> Result<(ExecutionStores, WorkflowStores, Arc<dyn JobDispatchQueue>), WorkerRunError> {
    use nebula_storage::postgres::{
        PgExecutionStore, PgIdempotencyGuard, PgJobDispatchQueue, PgJournalReader,
        PgResumeTokenStore, PgWorkflowStore, PgWorkflowVersionStore, init_schema as pg_init_schema,
    };
    use sqlx::postgres::PgPoolOptions;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(dsn)
        .await
        .map_err(WorkerRunError::PostgresDatabase)?;

    // Apply the port-scoped DDL — idempotent (`CREATE TABLE IF NOT EXISTS`).
    pg_init_schema(&pool).await?;

    tracing::info!("Postgres schema applied (ADR-0095 D3/D5 durable backend)");

    // Every store clone shares the same `PgPool` `Arc`; the pool manages
    // connections internally (max_connections(8)).
    let execution_store = Arc::new(PgExecutionStore::new(pool.clone()));
    let journal_reader = Arc::new(PgJournalReader::new(pool.clone()));
    // NodeResult and Checkpoint have no PG implementation — they store
    // transient in-process data (node output slots and stateful checkpoints)
    // within a single execution lifetime. Same rationale as the SQLite path.
    let node_results = Arc::new(InMemoryNodeResultStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    tracing::warn!(
        "node-result and checkpoint stores are in-memory (not persisted across restarts); \
         crash-recovery re-executes affected nodes via the reclaim sweep — \
         authoritative execution state is the Postgres execution row"
    );
    let idempotency = Arc::new(PgIdempotencyGuard::new(pool.clone()));
    let resume_tokens = Arc::new(PgResumeTokenStore::new(pool.clone()));
    let workflow_store = Arc::new(PgWorkflowStore::new(pool.clone()));
    let versions_store = Arc::new(PgWorkflowVersionStore::new(pool.clone()));
    let queue = Arc::new(PgJobDispatchQueue::new(pool));

    let execution_stores = ExecutionStores {
        execution: execution_store,
        journal: journal_reader,
        node_results,
        checkpoints,
        idempotency,
        resume_tokens,
    };
    let workflow_stores = WorkflowStores {
        workflow: workflow_store,
        versions: versions_store,
    };

    Ok((execution_stores, workflow_stores, queue))
}

/// Fail-closed twin compiled when the `postgres` feature is absent.
///
/// Returns [`WorkerRunError::PostgresFeatureNotEnabled`] so the binary never
/// silently falls back to SQLite when the operator explicitly set
/// `NEBULA_WORKER_DATABASE_URL`. The `_dsn` parameter mirrors the postgres
/// twin's signature so the call site in `build_stores` compiles under both
/// feature states.
#[cfg(not(feature = "postgres"))]
async fn build_pg_stores(
    _dsn: &str,
) -> Result<(ExecutionStores, WorkflowStores, Arc<dyn JobDispatchQueue>), WorkerRunError> {
    Err(WorkerRunError::PostgresFeatureNotEnabled)
}

/// Async startup, claim-loop, and graceful shutdown.
///
/// All errors are returned as [`WorkerRunError`]; `main` converts them to
/// stderr lines + `process::exit(1)`.
pub(crate) async fn run() -> Result<(), WorkerRunError> {
    // Install tracing subscriber first so every subsequent step emits logs.
    // `RUST_LOG` drives the filter; `info` is the implied default.
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr) // logs → stderr; stdout stays clean for data
        .init();

    tracing::info!("nebula-worker (core flavor) starting");

    let config = WorkerConfig::from_env()?;

    // Log the active backend. Only emit `db_path` on the SQLite path — on the
    // Postgres path it is the ignored default "nebula-worker.db" and emitting
    // it would mislead operators into thinking the file is in use.
    if config.database_url.is_some() {
        tracing::info!(
            backend = "postgres",
            batch_size = ?config.batch_size,
            poll_interval_ms = ?config.poll_interval_ms,
            "worker config loaded"
        );
    } else {
        tracing::info!(
            backend = "sqlite",
            db_path = %config.db_path,
            batch_size = ?config.batch_size,
            poll_interval_ms = ?config.poll_interval_ms,
            "worker config loaded"
        );
    }

    // Build the store bundle — SQLite or Postgres depending on config.
    let (execution_stores, workflow_stores, queue) = build_stores(&config).await?;

    // Assemble the core-flavor builder (boots CorePlugin + wires into engine).
    // The returned MetricsRegistry is the same instance the engine's ActionRuntime
    // uses; forwarding it into the worker builder ensures orchestrator dispatch and
    // reclaim counters land in the same registry as engine counters so a single
    // scraper sees all telemetry.
    let (mut builder, metrics, plugin_key) = build_core_flavor_runtime(
        execution_stores,
        workflow_stores,
        queue,
        config.processor_id,
    )?;

    // Apply optional env-driven tuning before materialising the runtime.
    builder = builder.with_metrics(metrics);
    if let Some(n) = config.batch_size {
        builder = builder.with_batch_size(n);
    }
    if let Some(ms) = config.poll_interval_ms {
        builder = builder.with_poll_interval(Duration::from_millis(ms));
    }

    let runtime = builder.build()?;

    tracing::info!(
        plugin = %plugin_key,
        "core-flavor runtime ready — starting claim-loop"
    );

    // Wire graceful shutdown: SIGINT (Ctrl-C) and SIGTERM on Unix.
    let cancel = CancellationToken::new();
    let handle = runtime.spawn(cancel.clone());

    wait_for_shutdown_signal(cancel).await?;

    tracing::info!(
        "shutdown signal received; waiting for the claim-loop task to exit — \
         claimed-but-not-yet-acked jobs are recovered by the reclaim sweep on next boot"
    );
    handle.await.expect(
        "worker task must not panic: a panic here indicates a bug in the claim-loop implementation",
    );
    tracing::info!("nebula-worker (core flavor) stopped cleanly");

    Ok(())
}

/// Wait for SIGINT (Ctrl-C) or SIGTERM, then cancel `token`.
async fn wait_for_shutdown_signal(token: CancellationToken) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }
    token.cancel();
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // ── Fail-closed routing test ──────────────────────────────────────────────
    //
    // Runs under the DEFAULT (no-postgres) feature set — the same path CI uses.
    // Proves two routing invariants in one shot:
    //
    //   1. `database_url = Some(dsn)` routes to the Postgres arm (not SQLite).
    //   2. Without `--features postgres`, that arm returns
    //      `WorkerRunError::PostgresFeatureNotEnabled` — never falls back to
    //      SQLite silently.
    //
    // Red-able: if `build_stores` ignored `database_url` and fell through to the
    // SQLite arm, it would attempt to open
    // "nebula-worker-MUST-NOT-BE-OPENED.db" and return `Ok(...)` or
    // `Err(SqliteDatabase(_))` — not `PostgresFeatureNotEnabled` — causing the
    // `matches!` assertion to fail.

    /// DSN set on a no-postgres binary must fail closed, never silently open SQLite.
    #[cfg(not(feature = "postgres"))]
    #[tokio::test]
    async fn database_url_set_without_postgres_feature_is_fail_closed() {
        use super::{WorkerConfig, WorkerRunError, build_stores};

        let config = WorkerConfig {
            database_url: Some("postgres://localhost/nebula_test".to_owned()),
            // A file name that would be obviously wrong if SQLite opened it.
            db_path: "nebula-worker-MUST-NOT-BE-OPENED.db".to_owned(),
            processor_id: [0u8; 16],
            batch_size: None,
            poll_interval_ms: None,
        };

        let result = build_stores(&config).await;

        assert!(
            matches!(result, Err(WorkerRunError::PostgresFeatureNotEnabled)),
            "expected WorkerRunError::PostgresFeatureNotEnabled, got: {result:?}"
        );
    }
}
