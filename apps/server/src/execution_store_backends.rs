//! Backend-specific construction for the server execution authority.

use std::{sync::Arc, time::Duration};

use nebula_api::{ApiConfig, config::ExecutionBackendKind};
use nebula_metrics::MetricsRegistry;

use crate::compose::{ExecutionStoreBundle, TransportInitError};
#[cfg(feature = "runtime-repair-red")]
use crate::compose::{ProfileBackendLifecycle, WorkerStoreProjection};

pub(crate) async fn build_execution_stores(
    api_config: &ApiConfig,
    explicit_postgres_dsn: Option<&str>,
    metrics: &MetricsRegistry,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    match api_config.execution.backend {
        ExecutionBackendKind::Memory => {
            warn_memory_outside_dev();
            build_memory_execution_stores(metrics)
        },
        ExecutionBackendKind::Sqlite => build_sqlite_execution_stores(api_config, metrics).await,
        ExecutionBackendKind::Postgres => {
            build_postgres_execution_stores(explicit_postgres_dsn, metrics).await
        },
        _ => unreachable!(
            "unrecognised ExecutionBackendKind variant; add its execution-store composition"
        ),
    }
}

fn build_memory_execution_stores(
    _metrics: &MetricsRegistry,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    #[cfg(feature = "runtime-repair-red")]
    use nebula_storage::inmem::{
        InMemoryCheckpointStore, InMemoryIdempotencyGuard, InMemoryOperationLedger,
    };
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryStartAcceptanceStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };

    let execution_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&execution_store);
    let journal = InMemoryJournalReader::new(&execution_store);
    let start_acceptance = Arc::new(InMemoryStartAcceptanceStore::new(&execution_store));
    let resume_token_store = execution_store.resume_token_store();
    let resume_producer = execution_store.resume_producer();
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store =
        InMemoryWorkflowStore::new_with_versions(&workflow_versions, &execution_store);
    let shared_control_queue: Arc<dyn nebula_storage_port::store::ControlQueue> =
        Arc::new(control_queue);
    let turn_handoff: Arc<dyn nebula_storage_port::store::ExecutionTurnHandoff> = Arc::new(
        nebula_storage::inmem::InMemoryTurnHandoff::new(&execution_store),
    );

    #[cfg(feature = "runtime-repair-red")]
    let worker_projection = {
        let projected_execution: Arc<dyn nebula_storage_port::store::ExecutionStore> =
            Arc::new(execution_store.clone());
        let projected_journal: Arc<dyn nebula_storage_port::store::ExecutionJournalReader> =
            Arc::new(journal.clone());
        let projected_node_results: Arc<dyn nebula_storage_port::store::NodeResultStore> =
            Arc::new(node_results.clone());
        let projected_resume_tokens: Arc<dyn nebula_storage_port::store::ResumeTokenStore> =
            Arc::new(resume_token_store.clone());
        WorkerStoreProjection {
            bundles: Arc::new(InMemoryStartAcceptanceStore::new(&execution_store)),
            metrics: MetricsRegistry::new(),
            revision_catalog: Arc::new(nebula_storage::InMemoryPlanFlavorCatalog::new(
                &execution_store,
            )),
            execution_stores: nebula_engine::ExecutionStores {
                execution: projected_execution,
                journal: projected_journal,
                node_results: projected_node_results,
                checkpoints: Arc::new(InMemoryCheckpointStore::new()),
                idempotency: Arc::new(InMemoryIdempotencyGuard::new()),
                resume_tokens: projected_resume_tokens,
                operation_ledger: Arc::new(InMemoryOperationLedger::new(&execution_store)),
            },
            control_queue: Arc::clone(&shared_control_queue),
            turn_handoff: Arc::clone(&turn_handoff),
            turn_recovery: Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(
                &execution_store,
            )),
        }
    };

    tracing::info!(
        backend = "memory",
        "execution-stores: in-memory adapters wired"
    );
    let revision_catalog = Arc::new(execution_store.plan_flavor_catalog());
    Ok(ExecutionStoreBundle {
        revision_catalog: revision_catalog.clone(),
        revision_installer: revision_catalog,
        workflow_store: Arc::new(workflow_store),
        workflow_version_store: Arc::new(workflow_versions),
        execution_store: Arc::new(execution_store),
        node_result_store: Arc::new(node_results),
        journal_reader: Arc::new(journal),
        control_queue: Arc::clone(&shared_control_queue),
        start_acceptance: start_acceptance.clone(),
        turn_handoff,
        start_reservation_maintenance: start_acceptance,
        resume_token_store: Arc::new(resume_token_store),
        resume_producer: Arc::new(resume_producer),
        #[cfg(feature = "runtime-repair-red")]
        worker_projection,
        #[cfg(feature = "runtime-repair-red")]
        backend_lifecycle: ProfileBackendLifecycle::Memory,
    })
}

fn warn_memory_outside_dev() {
    let environment = std::env::var("NEBULA_ENV").unwrap_or_default();
    if !matches!(environment.as_str(), "development" | "dev" | "local") {
        tracing::warn!(
            backend = "memory",
            nebula_env = %environment,
            component = "execution-stores",
            "execution-stores: in-memory adapters selected — execution state is lost \
             on restart and cannot be shared across processes; \
             set API_EXECUTION_BACKEND=sqlite (single-process durable) or \
             API_EXECUTION_BACKEND=postgres (multi-process) for production"
        );
    }
}

async fn build_sqlite_execution_stores(
    api_config: &ApiConfig,
    metrics: &MetricsRegistry,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    use nebula_storage::InMemoryNodeResultStore;
    use nebula_storage::sqlite::{
        SqliteControlQueue, SqliteExecutionStore, SqliteJournalReader, SqliteResumeProducer,
        SqliteResumeTokenStore, SqliteStartAcceptanceStore, SqliteTurnHandoff, SqliteWorkflowStore,
        SqliteWorkflowVersionStore, init_schema,
    };
    #[cfg(feature = "runtime-repair-red")]
    use nebula_storage::sqlite::{SqliteIdempotencyGuard, SqliteOperationLedger};
    use sqlx::sqlite::{
        SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
    };

    let database_path = &api_config.execution.db_path;
    let connection_options = SqliteConnectOptions::new()
        .filename(database_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connection_options)
        .await
        .map_err(|error| {
            TransportInitError::ExecutionDatabase(format!(
                "SQLite: failed to open '{database_path}': {error}"
            ))
        })?;
    init_schema(&pool).await.map_err(|error| {
        TransportInitError::ExecutionDatabase(format!(
            "SQLite: schema init failed for '{database_path}': {error}"
        ))
    })?;

    tracing::info!(backend = "sqlite", db_path = %database_path, "execution-stores: SQLite migrations ready");
    tracing::warn!(
        "node-result and checkpoint stores are in-memory (not persisted across restarts); \
         crash-recovery re-executes affected nodes via the reclaim sweep — \
         authoritative execution state is the SQLite execution row"
    );
    let node_results = Arc::new(InMemoryNodeResultStore::new());
    let workflow_store: Arc<dyn nebula_storage_port::store::WorkflowStore> =
        Arc::new(SqliteWorkflowStore::new(pool.clone()));
    let workflow_version_store: Arc<dyn nebula_storage_port::store::WorkflowVersionStore> =
        Arc::new(SqliteWorkflowVersionStore::new(pool.clone()));
    let execution_store: Arc<dyn nebula_storage_port::store::ExecutionStore> =
        Arc::new(SqliteExecutionStore::new(pool.clone()));
    let journal_reader: Arc<dyn nebula_storage_port::store::ExecutionJournalReader> =
        Arc::new(SqliteJournalReader::new(pool.clone()));
    let resume_token_store: Arc<dyn nebula_storage_port::store::ResumeTokenStore> =
        Arc::new(SqliteResumeTokenStore::new(pool.clone()));
    let control_queue: Arc<dyn nebula_storage_port::store::ControlQueue> =
        Arc::new(SqliteControlQueue::new(pool.clone()));
    let turn_handoff: Arc<dyn nebula_storage_port::store::ExecutionTurnHandoff> =
        Arc::new(SqliteTurnHandoff::new(pool.clone()));

    #[cfg(feature = "runtime-repair-red")]
    let worker_projection = {
        let worker_metrics = MetricsRegistry::new();
        WorkerStoreProjection {
            bundles: Arc::new(SqliteStartAcceptanceStore::new(pool.clone())),
            metrics: worker_metrics.clone(),
            revision_catalog: Arc::new(nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
                pool.clone(),
                &worker_metrics,
            )),
            execution_stores: nebula_engine::ExecutionStores {
                execution: Arc::clone(&execution_store),
                journal: Arc::clone(&journal_reader),
                node_results: Arc::clone(&node_results) as _,
                checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
                idempotency: Arc::new(SqliteIdempotencyGuard::new(pool.clone())),
                resume_tokens: Arc::clone(&resume_token_store),
                operation_ledger: Arc::new(SqliteOperationLedger::new(pool.clone())),
            },
            control_queue: Arc::clone(&control_queue),
            turn_handoff: Arc::clone(&turn_handoff),
            turn_recovery: Arc::new(SqliteTurnHandoff::new(pool.clone())),
        }
    };
    #[cfg(feature = "runtime-repair-red")]
    let backend_lifecycle = ProfileBackendLifecycle::Sqlite(pool.clone());
    let revision_catalog = Arc::new(nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
        pool.clone(),
        metrics,
    ));
    let start_acceptance = Arc::new(SqliteStartAcceptanceStore::new(pool.clone()));

    Ok(ExecutionStoreBundle {
        revision_catalog: revision_catalog.clone(),
        revision_installer: revision_catalog,
        workflow_store,
        workflow_version_store,
        execution_store,
        node_result_store: node_results,
        journal_reader,
        control_queue,
        start_acceptance: start_acceptance.clone(),
        turn_handoff,
        start_reservation_maintenance: start_acceptance,
        resume_token_store,
        resume_producer: Arc::new(SqliteResumeProducer::new(pool)),
        #[cfg(feature = "runtime-repair-red")]
        worker_projection,
        #[cfg(feature = "runtime-repair-red")]
        backend_lifecycle,
    })
}

#[cfg(feature = "postgres")]
async fn build_postgres_execution_stores(
    explicit_postgres_dsn: Option<&str>,
    metrics: &MetricsRegistry,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    use nebula_storage::InMemoryNodeResultStore;
    use nebula_storage::postgres::{
        PgControlQueue, PgExecutionStore, PgJournalReader, PgResumeProducer, PgResumeTokenStore,
        PgStartAcceptanceStore, PgTurnHandoff, PgWorkflowStore, PgWorkflowVersionStore,
        init_schema,
    };
    #[cfg(feature = "runtime-repair-red")]
    use nebula_storage::postgres::{PgIdempotencyGuard, PgOperationLedger};
    use sqlx::postgres::PgPoolOptions;

    let environment_dsn;
    let database_dsn = if let Some(explicit_dsn) = explicit_postgres_dsn {
        explicit_dsn
    } else {
        environment_dsn = std::env::var("DATABASE_URL").map_err(|_| {
            TransportInitError::ExecutionBackendUnavailable {
                requested: "postgres",
                requirement: "DATABASE_URL must be set when API_EXECUTION_BACKEND=postgres",
            }
        })?;
        &environment_dsn
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(database_dsn)
        .await
        .map_err(|error| {
            TransportInitError::ExecutionDatabase(format!(
                "Postgres: failed to connect to DATABASE_URL for execution stores: {error}"
            ))
        })?;
    init_schema(&pool).await.map_err(|error| {
        TransportInitError::ExecutionDatabase(format!(
            "Postgres: execution-store schema init failed: {error}"
        ))
    })?;

    tracing::info!(
        backend = "postgres",
        "execution-stores: Postgres migrations ready"
    );
    tracing::warn!(
        "node-result and checkpoint stores are in-memory (not persisted across restarts); \
         crash-recovery re-executes affected nodes via the reclaim sweep — \
         authoritative execution state is the Postgres execution row"
    );
    let workflow_store: Arc<dyn nebula_storage_port::store::WorkflowStore> =
        Arc::new(PgWorkflowStore::new(pool.clone()));
    let workflow_version_store: Arc<dyn nebula_storage_port::store::WorkflowVersionStore> =
        Arc::new(PgWorkflowVersionStore::new(pool.clone()));
    let execution_store: Arc<dyn nebula_storage_port::store::ExecutionStore> =
        Arc::new(PgExecutionStore::new(pool.clone()));
    let node_result_store: Arc<dyn nebula_storage_port::store::NodeResultStore> =
        Arc::new(InMemoryNodeResultStore::new());
    let journal_reader: Arc<dyn nebula_storage_port::store::ExecutionJournalReader> =
        Arc::new(PgJournalReader::new(pool.clone()));
    let resume_token_store: Arc<dyn nebula_storage_port::store::ResumeTokenStore> =
        Arc::new(PgResumeTokenStore::new(pool.clone()));
    let control_queue: Arc<dyn nebula_storage_port::store::ControlQueue> =
        Arc::new(PgControlQueue::new(pool.clone()));
    let turn_handoff: Arc<dyn nebula_storage_port::store::ExecutionTurnHandoff> =
        Arc::new(PgTurnHandoff::new(pool.clone()));

    #[cfg(feature = "runtime-repair-red")]
    let worker_projection = {
        let worker_metrics = MetricsRegistry::new();
        WorkerStoreProjection {
            bundles: Arc::new(PgStartAcceptanceStore::new(pool.clone())),
            metrics: worker_metrics.clone(),
            revision_catalog: Arc::new(nebula_storage::postgres::PgPlanFlavorCatalog::new(
                pool.clone(),
                &worker_metrics,
            )),
            execution_stores: nebula_engine::ExecutionStores {
                execution: Arc::clone(&execution_store),
                journal: Arc::clone(&journal_reader),
                node_results: Arc::clone(&node_result_store),
                checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
                idempotency: Arc::new(PgIdempotencyGuard::new(pool.clone())),
                resume_tokens: Arc::clone(&resume_token_store),
                operation_ledger: Arc::new(PgOperationLedger::new(pool.clone())),
            },
            control_queue: Arc::clone(&control_queue),
            turn_handoff: Arc::clone(&turn_handoff),
            turn_recovery: Arc::new(PgTurnHandoff::new(pool.clone())),
        }
    };
    #[cfg(feature = "runtime-repair-red")]
    let backend_lifecycle = ProfileBackendLifecycle::Postgres(pool.clone());
    let revision_catalog = Arc::new(nebula_storage::postgres::PgPlanFlavorCatalog::new(
        pool.clone(),
        metrics,
    ));
    let start_acceptance = Arc::new(PgStartAcceptanceStore::new(pool.clone()));

    Ok(ExecutionStoreBundle {
        revision_catalog: revision_catalog.clone(),
        revision_installer: revision_catalog,
        workflow_store,
        workflow_version_store,
        execution_store,
        node_result_store,
        journal_reader,
        control_queue,
        start_acceptance: start_acceptance.clone(),
        turn_handoff,
        start_reservation_maintenance: start_acceptance,
        resume_token_store,
        resume_producer: Arc::new(PgResumeProducer::new(pool)),
        #[cfg(feature = "runtime-repair-red")]
        worker_projection,
        #[cfg(feature = "runtime-repair-red")]
        backend_lifecycle,
    })
}

#[cfg(not(feature = "postgres"))]
async fn build_postgres_execution_stores(
    _explicit_postgres_dsn: Option<&str>,
    _metrics: &MetricsRegistry,
) -> Result<ExecutionStoreBundle, TransportInitError> {
    Err(TransportInitError::ExecutionBackendUnavailable {
        requested: "postgres",
        requirement: "build with `nebula-api/postgres` cargo feature to link sqlx + Pg execution stores",
    })
}
