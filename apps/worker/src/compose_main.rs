//! Async startup logic for the core-flavor worker binary.
//!
//! Separated from `main.rs` so the shutdown path and store wiring are testable
//! without starting the full `#[tokio::main]` harness. The `run` function is the
//! single entry point called by `main`.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage::sqlite::{
    SqliteExecutionStore, SqliteIdempotencyGuard, SqliteJobDispatchQueue, SqliteJournalReader,
    SqliteWorkflowStore, SqliteWorkflowVersionStore, init_schema,
};
use nebula_storage::{InMemoryCheckpointStore, InMemoryNodeResultStore};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, fmt};

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
    #[error(
        "database connection failed — check NEBULA_WORKER_DB_PATH and directory permissions: {0}"
    )]
    Database(#[from] sqlx::Error),

    /// Schema DDL (`CREATE TABLE IF NOT EXISTS`) failed.
    #[error("schema init failed — the SQLite file may be corrupt or read-only: {0}")]
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

    tracing::info!(
        db_path = %config.db_path,
        batch_size = ?config.batch_size,
        poll_interval_ms = ?config.poll_interval_ms,
        "worker config loaded"
    );

    // Build the SQLite pool (max 1 connection — SQLite single-writer contract).
    //
    // `BEGIN IMMEDIATE` CAS + claim-fencing in the store are only correct when a
    // single connection serialises all writes. WAL + NORMAL synchronous gives
    // read/write concurrency without fsync on every commit; busy_timeout prevents
    // instant SQLITE_BUSY errors if another tool (e.g. a CLI probe) briefly holds
    // the WAL write lock.
    //
    // **This SQLite file is not shareable across processes or hosts.** For
    // multi-worker deployments, configure the Postgres backend (future) and share
    // the connection string via the environment.
    //
    // **Windows note:** `tokio::signal::unix` is compiled out on Windows; only
    // Ctrl-C (SIGINT equivalent) triggers graceful shutdown.
    let opts = SqliteConnectOptions::new()
        .filename(&config.db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;

    // Apply the port-scoped DDL — idempotent `CREATE TABLE IF NOT EXISTS`.
    init_schema(&pool).await?;

    tracing::info!(db_path = %config.db_path, "SQLite schema applied");

    // Build the durable store bundle.
    //
    // Every store clone holds the same `SqlitePool` `Arc`; single max_connections
    // serialises writes at the connection level.
    let execution_store = Arc::new(SqliteExecutionStore::new(pool.clone()));
    let journal_reader = Arc::new(SqliteJournalReader::new(pool.clone()));
    // NodeResultStore and CheckpointStore have no SQLite-backed implementation yet;
    // both store transient in-process data (node output slots and stateful
    // checkpoints) that are written and read within a single execution lifetime.
    // Durability is provided by the ExecutionStore single-JSON-blob state machine,
    // not by these auxiliary stores. On a crash, the reclaim sweep re-delivers the
    // job and the engine re-executes the affected nodes from the last persisted state.
    let node_results = Arc::new(InMemoryNodeResultStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    tracing::warn!(
        "node-result and checkpoint stores are in-memory (not persisted across restarts); \
         crash-recovery re-executes affected nodes via the reclaim sweep — \
         authoritative execution state is the SQLite execution row"
    );
    let idempotency = Arc::new(SqliteIdempotencyGuard::new(pool.clone()));
    let workflow_store = Arc::new(SqliteWorkflowStore::new(pool.clone()));
    let versions_store = Arc::new(SqliteWorkflowVersionStore::new(pool.clone()));
    let queue = Arc::new(SqliteJobDispatchQueue::new(pool));

    let execution_stores = nebula_engine::ExecutionStores {
        execution: execution_store,
        journal: journal_reader,
        node_results,
        checkpoints,
        idempotency,
    };
    let workflow_stores = nebula_engine::WorkflowStores {
        workflow: workflow_store,
        versions: versions_store,
    };

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
