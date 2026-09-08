//! Async startup logic for the core-flavor worker binary.
//!
//! Separated from `main.rs` so the shutdown path and store wiring are testable
//! without starting the full `#[tokio::main]` harness. The `run` function is the
//! single entry point called by `main`.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage::sqlite::{
    SqliteControlQueue, SqliteExecutionStore, SqliteIdempotencyGuard, SqliteJournalReader,
    SqliteOperationLedger, SqliteResumeTokenStore, SqliteTurnHandoff, init_schema,
};
use nebula_storage::{InMemoryCheckpointStore, InMemoryNodeResultStore};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{EnvFilter, fmt};

use nebula_engine::ExecutionStores;
use nebula_storage_port::store::{ControlQueue, ExecutionTurnHandoff, TurnRecovery};
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

    /// A supervised worker component ended abnormally.
    ///
    /// Surfaced as a non-zero exit rather than logged and swallowed: a worker
    /// whose control consumer died is no longer draining work.
    #[error("worker runtime ended abnormally: {0}")]
    RuntimeStopped(#[from] nebula_worker::WorkerRuntimeError),

    /// The Tokio task hosting the supervised runtime panicked or was cancelled.
    #[error("worker runtime task ended abnormally: {0}")]
    RuntimeTask(#[source] tokio::task::JoinError),

    /// The runtime stopped without a shutdown request or a reported failure.
    #[error("worker runtime stopped before a shutdown signal")]
    RuntimeExited,

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

    /// Canonical schema admission or ordered migration failed.
    #[error("schema setup failed — the database may be unsupported or unavailable: {0}")]
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
    metrics: &nebula_metrics::MetricsRegistry,
) -> Result<
    (
        ExecutionStores,
        Arc<dyn ControlQueue>,
        Arc<dyn ExecutionTurnHandoff>,
        Arc<dyn TurnRecovery>,
        Arc<dyn nebula_storage_port::PlanFlavorCatalog>,
        Arc<dyn nebula_storage_port::store::StartAcceptanceStore>,
    ),
    WorkerRunError,
> {
    if let Some(dsn) = config.database_url.as_deref() {
        return build_pg_stores(dsn, metrics).await;
    }

    // SQLite is the single-process default. One connection serializes writes;
    // multi-process deployments select Postgres above.
    let options = SqliteConnectOptions::new()
        .filename(&config.db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(WorkerRunError::SqliteDatabase)?;
    init_schema(&pool).await?;
    tracing::info!(db_path = %config.db_path, "SQLite migrations ready");

    let execution_store = Arc::new(SqliteExecutionStore::new(pool.clone()));
    let journal_reader = Arc::new(SqliteJournalReader::new(pool.clone()));
    let node_results = Arc::new(InMemoryNodeResultStore::new());
    let checkpoints = Arc::new(InMemoryCheckpointStore::new());
    tracing::warn!(
        "node-result and checkpoint stores are in-memory; authoritative execution state is SQLite"
    );
    let idempotency = Arc::new(SqliteIdempotencyGuard::new(pool.clone()));
    let resume_tokens = Arc::new(SqliteResumeTokenStore::new(pool.clone()));
    let turn_handoff = Arc::new(SqliteTurnHandoff::new(pool.clone()));
    let catalog = Arc::new(nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
        pool.clone(),
        metrics,
    ));
    let bundles = Arc::new(nebula_storage::sqlite::SqliteStartAcceptanceStore::new(
        pool.clone(),
    ));
    let control_queue = Arc::new(SqliteControlQueue::new(pool.clone()));
    let execution_stores = ExecutionStores {
        execution: execution_store,
        journal: journal_reader,
        node_results,
        checkpoints,
        idempotency,
        resume_tokens,
        operation_ledger: Arc::new(SqliteOperationLedger::new(pool.clone())),
    };
    Ok((
        execution_stores,
        control_queue,
        turn_handoff.clone(),
        turn_handoff,
        catalog,
        bundles,
    ))
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
    metrics: &nebula_metrics::MetricsRegistry,
) -> Result<
    (
        ExecutionStores,
        Arc<dyn ControlQueue>,
        Arc<dyn ExecutionTurnHandoff>,
        Arc<dyn TurnRecovery>,
        Arc<dyn nebula_storage_port::PlanFlavorCatalog>,
        Arc<dyn nebula_storage_port::store::StartAcceptanceStore>,
    ),
    WorkerRunError,
> {
    use nebula_storage::postgres::{
        PgControlQueue, PgExecutionStore, PgIdempotencyGuard, PgJournalReader, PgOperationLedger,
        PgResumeTokenStore, PgTurnHandoff, init_schema as pg_init_schema,
    };
    use sqlx::postgres::PgPoolOptions;

    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(dsn)
        .await
        .map_err(WorkerRunError::PostgresDatabase)?;

    // Admit the canonical prefix and apply the ordered PostgreSQL migrations.
    pg_init_schema(&pool).await?;

    tracing::info!("Postgres migrations ready");

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
    // Same pool as the execution store and control queue — see the SQLite arm.
    let turn_handoff = Arc::new(PgTurnHandoff::new(pool.clone()));
    // Same pool as the execution store — see the SQLite arm.
    let catalog = Arc::new(nebula_storage::postgres::PgPlanFlavorCatalog::new(
        pool.clone(),
        metrics,
    ));
    let bundles = Arc::new(nebula_storage::postgres::PgStartAcceptanceStore::new(
        pool.clone(),
    ));
    let control_queue = Arc::new(PgControlQueue::new(pool.clone()));

    let execution_stores = ExecutionStores {
        execution: execution_store,
        journal: journal_reader,
        node_results,
        checkpoints,
        idempotency,
        resume_tokens,
        operation_ledger: Arc::new(PgOperationLedger::new(pool.clone())),
    };
    Ok((
        execution_stores,
        control_queue,
        turn_handoff.clone(),
        turn_handoff,
        catalog,
        bundles,
    ))
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
    _metrics: &nebula_metrics::MetricsRegistry,
) -> Result<
    (
        ExecutionStores,
        Arc<dyn ControlQueue>,
        Arc<dyn ExecutionTurnHandoff>,
        Arc<dyn TurnRecovery>,
        Arc<dyn nebula_storage_port::PlanFlavorCatalog>,
        Arc<dyn nebula_storage_port::store::StartAcceptanceStore>,
    ),
    WorkerRunError,
> {
    Err(WorkerRunError::PostgresFeatureNotEnabled)
}

/// Async startup, durable worker processing, and graceful shutdown.
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
        tracing::info!(backend = "postgres", "worker config loaded");
    } else {
        tracing::info!(
            backend = "sqlite",
            db_path = %config.db_path,
            "worker config loaded"
        );
    }

    // Build the store bundle — SQLite or Postgres depending on config.
    let metrics = nebula_metrics::MetricsRegistry::new();
    let (execution_stores, control_queue, turn_handoff, turn_recovery, catalog, bundles) =
        build_stores(&config, &metrics).await?;

    // Assemble the core-flavor builder (boots CorePlugin + wires into engine).
    let (builder, _metrics, plugin_key) = build_core_flavor_runtime(
        execution_stores,
        turn_handoff,
        turn_recovery,
        config.processor_id,
        nebula_worker_bin::compose::CoreFlavorRevisionInputs {
            metrics,
            artifact_set_digest: config.artifact_set_digest,
            catalog,
            bundles,
        },
    )?;

    let runtime = builder.with_control_queue(control_queue).build()?;

    tracing::info!(
        plugin = %plugin_key,
        "core-flavor runtime ready"
    );

    // Wire graceful shutdown: SIGINT (Ctrl-C) and SIGTERM on Unix.
    let cancel = CancellationToken::new();
    let mut handle = runtime.spawn(cancel.clone());

    tokio::select! {
        signal = wait_for_shutdown_signal() => {
            signal?;
            cancel.cancel();
            tracing::info!(
                "shutdown signal received; waiting for the worker runtime to exit"
            );
            handle.await.map_err(WorkerRunError::RuntimeTask)??;
        },
        runtime_result = &mut handle => {
            cancel.cancel();
            runtime_result.map_err(WorkerRunError::RuntimeTask)??;
            return Err(WorkerRunError::RuntimeExited);
        },
    }
    tracing::info!("nebula-worker (core flavor) stopped cleanly");

    Ok(())
}

/// Wait for SIGINT (Ctrl-C) or SIGTERM, then cancel `token`.
async fn wait_for_shutdown_signal() -> Result<(), std::io::Error> {
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
            artifact_set_digest: nebula_core::ArtifactSetDigest::from_bytes([0x71; 32]),
            database_url: Some("postgres://localhost/nebula_test".to_owned()),
            // A file name that would be obviously wrong if SQLite opened it.
            db_path: "nebula-worker-MUST-NOT-BE-OPENED.db".to_owned(),
            processor_id: [0u8; 16],
        };

        let result = build_stores(&config, &nebula_metrics::MetricsRegistry::new()).await;

        assert!(
            matches!(result, Err(WorkerRunError::PostgresFeatureNotEnabled)),
            "expected WorkerRunError::PostgresFeatureNotEnabled, got: {result:?}"
        );
    }
}
