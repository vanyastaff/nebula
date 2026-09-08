use std::sync::Arc;

use nebula_engine::{ExecutionStores, WorkflowStores};
use nebula_metrics::MetricsRegistry;
use nebula_storage_port::store::{
    OperationLedger, PlanFlavorCatalog, PlanFlavorCatalogWriter, StartAcceptanceStore,
};

pub(super) struct Ports {
    pub stores: ExecutionStores,
    pub workflows: WorkflowStores,
    pub starts: Arc<dyn StartAcceptanceStore>,
    pub ledger: Arc<dyn OperationLedger>,
    pub catalog: Arc<dyn PlanFlavorCatalog>,
    pub writer: Arc<dyn PlanFlavorCatalogWriter>,
}

impl Ports {
    pub(super) fn memory() -> Self {
        Self::memory_core(Arc::new(nebula_storage::InMemoryExecutionStore::new()))
    }

    pub(super) fn memory_core(core: Arc<nebula_storage::InMemoryExecutionStore>) -> Self {
        use nebula_storage::*;
        let versions = Arc::new(InMemoryWorkflowVersionStore::new());
        let workflow = Arc::new(InMemoryWorkflowStore::new_with_versions(&versions, &core));
        let catalog = Arc::new(core.plan_flavor_catalog());
        let ledger = Arc::new(inmem::InMemoryOperationLedger::new(&core));
        Self {
            stores: ExecutionStores {
                execution: core.clone(),
                journal: Arc::new(InMemoryJournalReader::new(&core)),
                node_results: Arc::new(InMemoryNodeResultStore::new()),
                checkpoints: Arc::new(InMemoryCheckpointStore::new()),
                idempotency: Arc::new(InMemoryIdempotencyGuard::new()),
                resume_tokens: Arc::new(core.resume_token_store()),
                operation_ledger: ledger.clone(),
            },
            workflows: WorkflowStores { workflow, versions },
            starts: Arc::new(inmem::InMemoryStartAcceptanceStore::new(&core)),
            ledger,
            catalog: catalog.clone(),
            writer: catalog,
        }
    }

    pub(super) fn sqlite(pool: sqlx::SqlitePool) -> Self {
        use nebula_storage::sqlite::*;
        let catalog = Arc::new(SqlitePlanFlavorCatalog::new(
            pool.clone(),
            &MetricsRegistry::new(),
        ));
        let ledger = Arc::new(SqliteOperationLedger::new(pool.clone()));
        Self {
            stores: ExecutionStores {
                execution: Arc::new(SqliteExecutionStore::new(pool.clone())),
                journal: Arc::new(SqliteJournalReader::new(pool.clone())),
                node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
                checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
                idempotency: Arc::new(SqliteIdempotencyGuard::new(pool.clone())),
                resume_tokens: Arc::new(SqliteResumeTokenStore::new(pool.clone())),
                operation_ledger: ledger.clone(),
            },
            workflows: WorkflowStores {
                workflow: Arc::new(SqliteWorkflowStore::new(pool.clone())),
                versions: Arc::new(SqliteWorkflowVersionStore::new(pool.clone())),
            },
            starts: Arc::new(SqliteStartAcceptanceStore::new(pool)),
            ledger,
            catalog: catalog.clone(),
            writer: catalog,
        }
    }

    pub(super) fn postgres(pool: sqlx::PgPool) -> Self {
        use nebula_storage::postgres::*;
        let catalog = Arc::new(PgPlanFlavorCatalog::new(
            pool.clone(),
            &MetricsRegistry::new(),
        ));
        let ledger = Arc::new(PgOperationLedger::new(pool.clone()));
        Self {
            stores: ExecutionStores {
                execution: Arc::new(PgExecutionStore::new(pool.clone())),
                journal: Arc::new(PgJournalReader::new(pool.clone())),
                node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
                checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
                idempotency: Arc::new(PgIdempotencyGuard::new(pool.clone())),
                resume_tokens: Arc::new(PgResumeTokenStore::new(pool.clone())),
                operation_ledger: ledger.clone(),
            },
            workflows: WorkflowStores {
                workflow: Arc::new(PgWorkflowStore::new(pool.clone())),
                versions: Arc::new(PgWorkflowVersionStore::new(pool.clone())),
            },
            starts: Arc::new(PgStartAcceptanceStore::new(pool)),
            ledger,
            catalog: catalog.clone(),
            writer: catalog,
        }
    }
}
