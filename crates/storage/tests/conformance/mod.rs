//! Behavioral conformance harness for the storage port.
//!
//! One backend-agnostic contract suite (spec-16 §5 / §9) exercised across
//! `{InMemory, SQLite :memory:, Postgres (DATABASE_URL-gated)}`. Each
//! backend implements [`Backend`]; the shared assertions encode the
//! abstract concurrency + tenancy contract every adapter must satisfy:
//!
//! - create → get round-trip
//! - CAS conflict returns `VersionConflict { actual }`
//! - a stale fencing token returns `FencedOut`
//! - the atomic triple (state + outbox + journal) is all-or-nothing
//! - idempotency key shape + first-writer-wins
//! - cross-scope `get` / `commit` ⇒ `None` / `NotFound` (never another
//!   tenant's row)
//!
//! All three adapters (InMemory / SQLite / Postgres) implement the port.
//! The Postgres case is `DATABASE_URL`-gated and skip-cleans (WARN +
//! pass) when no database is configured; the SQLite case skips without
//! the `sqlite` feature. A skipped backend never reports a false green
//! and never hard-fails on a host that cannot run it.

use std::sync::Arc;

use nebula_core::{
    ExecutablePlanRevisionId, ExecutionContractBundleId, ExecutionId, OrgId, PluginKey,
    PluginSetId, WorkerFlavorRevisionId, WorkflowId, WorkflowVersionId, WorkspaceId,
};
use nebula_storage_port::dto::{
    CachedRecord, ContractBundleRecord, ControlCommand, ControlMsg, JobDispatchMsg, JournalEntry,
    MaterializedStart, NewExecution, ResumeTarget, WebhookActivationRecord, WebhookMode,
    WorkflowRecord, WorkflowVersionRecord,
};
use nebula_storage_port::store::{
    ClaimGeneration, ControlClaimToken, ControlQueue, ExecutionJournalReader, ExecutionStore,
    IdempotencyGuard, IdempotencyStore, JobClaimToken, JobDispatchQueue, StartAcceptanceStore,
    StartContractIdentity, StartMaterialization, WebhookActivationStore, WorkflowStore,
    WorkflowVersionStore,
};
use nebula_storage_port::{
    BeginDrainOutcome, ExecutionReferenceTransition, FencingToken, PlanFlavorCatalogAdmin,
    PlanFlavorCatalogWriter, PlanFlavorRevisionRecord, PlanFlavorRevisionTarget,
    RevisionInsertOutcome, RevisionRecordBytes, Scope, StorageError, TransitionBatch,
    TransitionOutcome, WorkerFlavorRevisionRecord,
};

/// A storage backend under conformance test. Returns port handles built on
/// that backend's concrete adapter.
#[async_trait::async_trait]
pub(crate) trait Backend: Send + Sync {
    /// Human-readable backend name (used in assertion messages).
    fn name(&self) -> &'static str;
    /// An execution store backed by this backend.
    async fn execution_store(&self) -> Arc<dyn ExecutionStore>;
    /// An idempotency guard backed by this backend.
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard>;
    /// A control-queue (durable outbox) backed by this backend, sharing
    /// the same store as [`Backend::execution_store`] so a `commit`'s
    /// outbox rows are observable through `claim_pending`.
    async fn control_queue(&self) -> Arc<dyn ControlQueue>;
    /// A journal reader backed by this backend, sharing the same store as
    /// [`Backend::execution_store`] so a `commit`'s journal entries are
    /// observable.
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader>;
    /// A durable idempotent-replay cache backed by this backend.
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore>;
    /// A webhook-activation store backed by this backend.
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore>;
    /// A workflow-row store backed by this backend (spec-16 split).
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore>;
    /// A workflow-version store backed by this backend, sharing the same
    /// backend as [`Backend::workflow_store`].
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore>;
    /// A job-dispatch queue backed by this backend.
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue>;
    /// A keyed-start acceptance store backed by this backend, sharing the same
    /// core as [`Backend::execution_store`] and [`Backend::control_queue`] so
    /// `materialize_start` commits every start-owned write together.
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore>;
    /// The exact plan/flavor catalog writer backed by this backend, sharing
    /// the same store as [`Backend::start_acceptance_store`] so a materialized
    /// start can install its contract revisions before acceptance.
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter>;
    /// The exact plan/flavor catalog lifecycle admin backed by this backend,
    /// used to prove a drain causes materialized starts to fail closed.
    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin>;
}

/// Test-only clock control for SQL job-dispatch retention assertions.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
#[async_trait::async_trait]
pub(crate) trait SqlJobTimestampFixture: Backend {
    /// Move a job's current processing timestamp into the past.
    async fn age_job_timestamp(
        &self,
        job_id: &[u8; 16],
        age: std::time::Duration,
    ) -> Result<(), StorageError>;
}

/// InMemory backend (always available).
///
/// Holds one execution store whose core is shared (it is `Clone` over an
/// `Arc<Mutex<…>>`), so the control queue, journal reader, and job-dispatch
/// queue observe the same rows under one lock.
pub(crate) struct InMemoryBackend {
    store: nebula_storage::inmem::InMemoryExecutionStore,
    guard: nebula_storage::inmem::InMemoryIdempotencyGuard,
    idem_store: nebula_storage::inmem::InMemoryIdempotencyStore,
    webhook: nebula_storage::inmem::InMemoryWebhookActivationStore,
    workflow: nebula_storage::inmem::InMemoryWorkflowStore,
    workflow_version: nebula_storage::inmem::InMemoryWorkflowVersionStore,
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        // The workflow-row store shares the version store's map (same
        // contract as `control_queue`/`journal` over the shared execution
        // core) so `save_with_published_version` is genuinely atomic
        // across the pair under the conformance matrix.
        let workflow_version = nebula_storage::inmem::InMemoryWorkflowVersionStore::new();
        let store = nebula_storage::inmem::InMemoryExecutionStore::new();
        let workflow = nebula_storage::inmem::InMemoryWorkflowStore::new_with_versions(
            &workflow_version,
            &store,
        );
        Self {
            store,
            guard: nebula_storage::inmem::InMemoryIdempotencyGuard::new(),
            idem_store: nebula_storage::inmem::InMemoryIdempotencyStore::new(),
            webhook: nebula_storage::inmem::InMemoryWebhookActivationStore::new(),
            workflow,
            workflow_version,
        }
    }
}

#[async_trait::async_trait]
impl Backend for InMemoryBackend {
    fn name(&self) -> &'static str {
        "InMemory"
    }
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(self.store.clone())
    }
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(self.guard.clone())
    }
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_storage::inmem::InMemoryControlQueue::new(
            &self.store,
        ))
    }
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_storage::inmem::InMemoryJournalReader::new(
            &self.store,
        ))
    }
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(self.idem_store.clone())
    }
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(self.webhook.clone())
    }
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(self.workflow.clone())
    }
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(self.workflow_version.clone())
    }
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        Arc::new(nebula_storage::inmem::InMemoryJobDispatchQueue::new(
            &self.store,
        ))
    }
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
            &self.store,
        ))
    }
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter> {
        Arc::new(nebula_storage::inmem::InMemoryPlanFlavorCatalog::new(
            &self.store,
        ))
    }
    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin> {
        Arc::new(nebula_storage::inmem::InMemoryPlanFlavorCatalog::new(
            &self.store,
        ))
    }
}

/// SQLite `:memory:` backend.
///
/// Each `Backend` instance owns one shared-cache in-memory database (so a
/// `create` and a later `commit`/`get` observe the same rows) created
/// lazily on first store request. Only built when the `sqlite` feature is
/// on; without it the case skips like Postgres.
#[derive(Default)]
pub(crate) struct SqliteBackend {
    #[cfg(feature = "sqlite")]
    pool: tokio::sync::OnceCell<sqlx::SqlitePool>,
}

#[cfg(feature = "sqlite")]
impl SqliteBackend {
    async fn pool(&self) -> sqlx::SqlitePool {
        use std::str::FromStr;
        self.pool
            .get_or_init(|| async {
                let db_name = format!("nebula-conformance-{}", uuid::Uuid::new_v4());
                let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
                let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&url)
                    .expect("parse sqlite memory url")
                    .create_if_missing(true);
                let pool = sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(opts)
                    .await
                    .expect("connect sqlite memory");
                nebula_storage::sqlite::init_schema(&pool)
                    .await
                    .expect("install port schema");
                pool
            })
            .await
            .clone()
    }
}

#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl SqlJobTimestampFixture for SqliteBackend {
    async fn age_job_timestamp(
        &self,
        job_id: &[u8; 16],
        age: std::time::Duration,
    ) -> Result<(), StorageError> {
        let age_ms = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
        let timestamp_ms = chrono::Utc::now().timestamp_millis().saturating_sub(age_ms);
        let rows_updated =
            sqlx::query("UPDATE port_job_dispatch_queue SET processed_at_ms = ? WHERE id = ?")
                .bind(timestamp_ms)
                .bind(job_id.as_slice())
                .execute(&self.pool().await)
                .await
                .map_err(|error| StorageError::Connection(error.to_string()))?
                .rows_affected();
        if rows_updated != 1 {
            return Err(StorageError::NotFound {
                entity: "job_dispatch",
                id: hex::encode(job_id),
            });
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Backend for SqliteBackend {
    fn name(&self) -> &'static str {
        "Sqlite(:memory:)"
    }
    #[cfg(feature = "sqlite")]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(nebula_storage::sqlite::SqliteExecutionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(nebula_storage::sqlite::SqliteIdempotencyGuard::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_storage::sqlite::SqliteControlQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_storage::sqlite::SqliteJournalReader::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(nebula_storage::sqlite::SqliteIdempotencyStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(nebula_storage::sqlite::SqliteWebhookActivationStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(nebula_storage::sqlite::SqliteWorkflowStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(nebula_storage::sqlite::SqliteWorkflowVersionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        Arc::new(nebula_storage::sqlite::SqliteJobDispatchQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        Arc::new(nebula_storage::sqlite::SqliteStartAcceptanceStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter> {
        Arc::new(nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
            self.pool().await,
            &nebula_metrics::MetricsRegistry::new(),
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
    #[cfg(feature = "sqlite")]
    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin> {
        Arc::new(nebula_storage::sqlite::SqlitePlanFlavorCatalog::new(
            self.pool().await,
            &nebula_metrics::MetricsRegistry::new(),
        ))
    }
    #[cfg(not(feature = "sqlite"))]
    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin> {
        unimplemented!("build with --features sqlite to exercise the SQLite backend")
    }
}

/// Postgres backend — only exercised when `DATABASE_URL` is set and the
/// crate is built with `--features postgres`; otherwise `skip_reason`
/// short-circuits the case so the suite stays green on a machine without
/// a database. Each `Backend` instance owns one pool created lazily on
/// first store request; the port schema is installed once.
///
/// The instance also owns a **private PostgreSQL schema**. `InMemoryBackend`
/// and `SqliteBackend` hand every case a fresh empty store, and the shared
/// assertions rely on that: they use fixed fixture ids (`wf_c`, `exe_cq`, …)
/// that several cases reuse. Pointing every case at one shared database
/// instead made those ids collide — the second case to run saw
/// `Duplicate { entity: "workflow", detail: "workflow wf_c already exists" }`
/// whether or not the cases ran concurrently. Because the Postgres case
/// skip-cleans without `DATABASE_URL`, and no CI job set one for this suite,
/// the collisions stayed invisible: the "shared oracle" was green precisely
/// because its Postgres arm never ran.
///
/// A per-instance schema restores the same-fresh-store contract the other two
/// backends already satisfy. The migration catalog observes and installs
/// through `current_schema()`, so it sees a genuinely fresh database here.
#[derive(Default)]
pub(crate) struct PostgresBackend {
    #[cfg(feature = "postgres")]
    pool: tokio::sync::OnceCell<sqlx::PgPool>,
}

#[cfg(feature = "postgres")]
#[path = "../support/postgres_schema.rs"]
mod postgres_schema;

#[cfg(feature = "postgres")]
impl PostgresBackend {
    async fn pool(&self) -> sqlx::PgPool {
        self.pool
            .get_or_init(|| async {
                let url = std::env::var("DATABASE_URL")
                    .unwrap_or_else(|e| panic!("DATABASE_URL required for the Postgres case: {e}"));
                let pool = postgres_schema::connect_with_private_schema(&url, "nebula_conformance")
                    .await
                    .expect("connect Postgres (DATABASE_URL)");
                nebula_storage::postgres::init_schema(&pool)
                    .await
                    .expect("install port schema");
                pool
            })
            .await
            .clone()
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl SqlJobTimestampFixture for PostgresBackend {
    async fn age_job_timestamp(
        &self,
        job_id: &[u8; 16],
        age: std::time::Duration,
    ) -> Result<(), StorageError> {
        let age_ms = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
        let timestamp_ms = chrono::Utc::now().timestamp_millis().saturating_sub(age_ms);
        let rows_updated =
            sqlx::query("UPDATE port_job_dispatch_queue SET processed_at_ms = $1 WHERE id = $2")
                .bind(timestamp_ms)
                .bind(job_id.as_slice())
                .execute(&self.pool().await)
                .await
                .map_err(|error| StorageError::Connection(error.to_string()))?
                .rows_affected();
        if rows_updated != 1 {
            return Err(StorageError::NotFound {
                entity: "job_dispatch",
                id: hex::encode(job_id),
            });
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Backend for PostgresBackend {
    fn name(&self) -> &'static str {
        "Postgres"
    }
    #[cfg(feature = "postgres")]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(nebula_storage::postgres::PgExecutionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(nebula_storage::postgres::PgIdempotencyGuard::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_storage::postgres::PgControlQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_storage::postgres::PgJournalReader::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(nebula_storage::postgres::PgIdempotencyStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(nebula_storage::postgres::PgWebhookActivationStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(nebula_storage::postgres::PgWorkflowStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(nebula_storage::postgres::PgWorkflowVersionStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        Arc::new(nebula_storage::postgres::PgJobDispatchQueue::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        Arc::new(nebula_storage::postgres::PgStartAcceptanceStore::new(
            self.pool().await,
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter> {
        Arc::new(nebula_storage::postgres::PgPlanFlavorCatalog::new(
            self.pool().await,
            &nebula_metrics::MetricsRegistry::new(),
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
    #[cfg(feature = "postgres")]
    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin> {
        Arc::new(nebula_storage::postgres::PgPlanFlavorCatalog::new(
            self.pool().await,
            &nebula_metrics::MetricsRegistry::new(),
        ))
    }
    #[cfg(not(feature = "postgres"))]
    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin> {
        unimplemented!("build with --features postgres to exercise the Postgres backend")
    }
}

mod requirements;

/// Postgres skip decision, resolved by feature flag so there is exactly
/// one match arm for the `"Postgres"` literal (avoids overlapping-pattern
/// lint when the feature is off).
#[cfg(feature = "postgres")]
fn postgres_skip() -> Option<&'static str> {
    match std::env::var("DATABASE_URL") {
        Ok(_) => None,
        Err(error) => {
            assert!(
                std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
                "NEBULA_REQUIRE_POSTGRES is set but DATABASE_URL is absent or non-Unicode"
            );
            match error {
                std::env::VarError::NotPresent => {
                    Some("DATABASE_URL unset; skipping Postgres case")
                },
                std::env::VarError::NotUnicode(_) => {
                    panic!("DATABASE_URL must be valid Unicode")
                },
            }
        },
    }
}

#[cfg(not(feature = "postgres"))]
fn postgres_skip() -> Option<&'static str> {
    assert!(
        std::env::var_os("NEBULA_REQUIRE_POSTGRES").is_none(),
        "NEBULA_REQUIRE_POSTGRES is set but the postgres feature is disabled"
    );
    Some("built without --features postgres; skipping Postgres case")
}

/// SQLite skip decision, resolved by feature flag (same single-arm
/// rationale as [`postgres_skip`]).
#[cfg(feature = "sqlite")]
fn sqlite_skip() -> Option<&'static str> {
    None
}

#[cfg(not(feature = "sqlite"))]
fn sqlite_skip() -> Option<&'static str> {
    Some("built without --features sqlite; skipping SQLite case")
}

/// Returns a skip reason for a backend whose prerequisites are not met, or
/// `None` if the case should run. Postgres skips without `DATABASE_URL` or
/// the `postgres` feature; SQLite skips without the `sqlite` feature.
/// Setting `NEBULA_REQUIRE_POSTGRES` makes unavailable Postgres a hard failure.
#[must_use]
pub(crate) fn skip_reason(backend: &dyn Backend) -> Option<&'static str> {
    match backend.name() {
        "Postgres" => postgres_skip(),
        "Sqlite(:memory:)" => sqlite_skip(),
        _ => None,
    }
}

fn scope_a() -> Scope {
    Scope::new("ws_a", "org_a")
}

fn scope_b() -> Scope {
    Scope::new("ws_b", "org_b")
}

// ── shared contract assertions ────────────────────────────────────────────

/// create → get returns the row within the same scope.
pub(crate) async fn assert_create_get_roundtrip(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_1", "wf_1", serde_json::json!({"k": 1}))
        .await
        .expect("create");
    let got = store.get(&s, "exe_1").await.expect("get");
    let rec = got.unwrap_or_else(|| panic!("[{}] expected the row", backend.name()));
    assert_eq!(rec.id, "exe_1");
    assert_eq!(rec.workflow_id, "wf_1");
}

/// A commit whose `expected_version` does not match the row returns
/// `VersionConflict { actual }`.
pub(crate) async fn assert_cas_conflict(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_cas", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(&s, "exe_cas", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease must be acquirable", backend.name()));
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_cas")
        .expected_version(999) // deliberately wrong
        .fencing(token)
        .new_state(serde_json::json!({"s": "running"}))
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(outcome, TransitionOutcome::VersionConflict { .. }),
        "[{}] expected VersionConflict, got {outcome:?}",
        backend.name()
    );
}

/// A commit carrying a superseded fencing token returns `FencedOut`.
pub(crate) async fn assert_stale_fencing_is_fenced_out(backend: &dyn Backend) -> serde_json::Value {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_fence", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let live = store
        .acquire_lease(
            &s,
            "exe_fence",
            "holder-1",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] live fence", backend.name()));
    let before = store
        .get(&s, "exe_fence")
        .await
        .expect("get before")
        .unwrap();
    // A token from an older generation than whatever the store now holds.
    let stale = FencingToken::from_generation(0);
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_fence")
        .expected_version(0)
        .fencing(stale)
        .new_state(serde_json::json!({"s": "running"}))
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(
            outcome,
            TransitionOutcome::FencedOut | TransitionOutcome::VersionConflict { .. }
        ),
        "[{}] a stale fencing token must not Apply, got {outcome:?}",
        backend.name()
    );
    let after = store
        .get(&s, "exe_fence")
        .await
        .expect("get after")
        .unwrap();
    assert_eq!(before, after, "stale fenced commit must make zero changes");
    serde_json::json!({
        "live_generation": live.generation(),
        "stale_generation": stale.generation(),
        "outcome": format!("{outcome:?}"),
        "version_before": before.version,
        "version_after": after.version,
        "state_before": before.state,
        "state_after": after.state,
        "stale_mutation_count": 0
    })
}

/// A live lease blocks every further `acquire_lease` — including a second
/// acquire by the *same* holder — and an acquire that follows a prior
/// (now-expired) lease bumps the fencing generation so the pre-expiry
/// token is dead. Zombie-runner closure: a live lease blocks re-acquire,
/// two concurrent runners must see exactly one winner, and a
/// crashed-then-restarted runner reusing its holder id cannot revive its
/// pre-crash token.
pub(crate) async fn assert_live_lease_blocks_acquire(backend: &dyn Backend) -> serde_json::Value {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_lease", "wf_1", serde_json::json!({}))
        .await
        .expect("create");

    let g1 = store
        .acquire_lease(
            &s,
            "exe_lease",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] first acquire must grant a token", backend.name()))
        .generation();

    // A second acquire while the lease is live is contention — even
    // for the SAME holder. Renewal is `renew_lease`, not a re-acquire.
    let contended = store
        .acquire_lease(
            &s,
            "exe_lease",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease");
    assert!(
        contended.is_none(),
        "[{}] a second acquire of a live lease (same holder) must be \
         contention (None), got {contended:?}",
        backend.name()
    );

    // After the lease expires, the same holder may re-acquire — but
    // the generation must strictly increase so the pre-expiry token is
    // fenced (the holder could be a zombie from before the crash).
    // Adapters floor the lease TTL to a 1s minimum (production never
    // wants sub-second leases), so acquire with a short TTL and sleep
    // past that floor before re-acquiring.
    store
        .create(&s, "exe_lease_z", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let z1 = store
        .acquire_lease(
            &s,
            "exe_lease_z",
            "holder",
            std::time::Duration::from_millis(1),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] zombie-case first acquire", backend.name()))
        .generation();
    // Let the floored (≈1s) lease expire.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let z2 = store
        .acquire_lease(
            &s,
            "exe_lease_z",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| {
            panic!(
                "[{}] same holder must re-acquire an expired lease",
                backend.name()
            )
        })
        .generation();
    assert!(
        z2 > z1,
        "[{}] re-acquire after expiry must bump the fencing generation \
         (z1={z1}, z2={z2}) so the pre-expiry token is fenced",
        backend.name()
    );
    // Sanity: the first execution's generation was monotone too.
    assert!(
        g1 <= z1.max(g1),
        "[{}] generations monotone",
        backend.name()
    );
    serde_json::json!({
        "first_generation": g1,
        "same_holder_live_reacquire_granted": contended.is_some(),
        "expired_generation": z1,
        "recovered_generation": z2,
        "recovered_generation_advanced": z2 > z1
    })
}

/// The atomic triple commits state + outbox + journal together; a reader
/// observes all three after a successful commit.
pub(crate) async fn assert_atomic_triple(backend: &dyn Backend) -> serde_json::Value {
    let store = backend.execution_store().await;
    let s = scope_a();
    store
        .create(&s, "exe_triple", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(
            &s,
            "exe_triple",
            "holder",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease", backend.name()));
    let msg = ControlMsg {
        id: [1u8; 16],
        execution_id: "exe_triple".into(),
        command: ControlCommand::Cancel,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let je = JournalEntry {
        seq: None,
        payload: serde_json::json!({"event": "transition"}),
    };
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_triple")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "running"}))
        .outbox(vec![msg])
        .journal(vec![je])
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "[{}] expected Applied, got {outcome:?}",
        backend.name()
    );
    let rec = store
        .get(&s, "exe_triple")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] row after commit", backend.name()));
    assert_eq!(
        rec.state,
        serde_json::json!({"s": "running"}),
        "[{}] state must reflect the committed transition",
        backend.name()
    );
    let journal = backend
        .journal_reader()
        .await
        .get_journal(&s, "exe_triple")
        .await
        .expect("read committed journal");
    assert_eq!(journal.len(), 1);
    assert_eq!(
        journal[0].payload,
        serde_json::json!({"event": "transition"})
    );
    let queue = backend.control_queue().await;
    let claims = queue
        .claim_pending(&[0xA1; 16], 1)
        .await
        .expect("claim committed outbox");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].msg.command, ControlCommand::Cancel);
    let command_id = claims[0].msg.id;
    queue
        .mark_completed(&claims[0].token)
        .await
        .expect("complete observed outbox");
    serde_json::json!({
        "transition_outcome": format!("{outcome:?}"),
        "execution_version": rec.version,
        "state": rec.state,
        "journal_entry_count": journal.len(),
        "journal_payload": journal[0].payload,
        "outbox_claim_count": claims.len(),
        "outbox_command": format!("{:?}", claims[0].msg.command),
        "outbox_command_id": command_id,
        "owner_fence_generation": token.generation()
    })
}

/// Idempotency key shape `{execution_id}:{node_id}:{attempt}` is
/// first-writer-wins: the first `check_and_mark` returns true, the second
/// false.
pub(crate) async fn assert_idempotency_first_writer_wins(backend: &dyn Backend) {
    let guard = backend.idempotency_guard().await;
    let s = scope_a();
    let first = guard
        .check_and_mark(&s, "exe_1", "node_1", 1)
        .await
        .expect("check_and_mark #1");
    let second = guard
        .check_and_mark(&s, "exe_1", "node_1", 1)
        .await
        .expect("check_and_mark #2");
    assert!(first, "[{}] first mark must win", backend.name());
    assert!(
        !second,
        "[{}] second mark on the same key must lose",
        backend.name()
    );
}

/// A `get` with a mismatched scope yields `Ok(None)` — never another
/// tenant's row, never an error that leaks existence.
pub(crate) async fn assert_cross_scope_get_is_none(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    store
        .create(&scope_a(), "exe_x", "wf_1", serde_json::json!({}))
        .await
        .expect("create in scope A");
    let miss = store.get(&scope_b(), "exe_x").await.expect("get");
    assert!(
        miss.is_none(),
        "[{}] cross-scope get must not leak the row",
        backend.name()
    );
}

/// A `commit` against an id that exists only in another tenant's scope
/// must not Apply (the row is invisible cross-tenant).
pub(crate) async fn assert_cross_scope_commit_is_rejected(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    store
        .create(&scope_a(), "exe_y", "wf_1", serde_json::json!({}))
        .await
        .expect("create in scope A");
    let batch = TransitionBatch::builder()
        .scope(scope_b()) // attacker's scope
        .execution_id("exe_y")
        .expected_version(0)
        .fencing(FencingToken::from_generation(0))
        .new_state(serde_json::json!({"s": "hijacked"}))
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await;
    // Any of VersionConflict / FencedOut / NotFound (Err) is an acceptable
    // rejection; the only forbidden outcome is a successful cross-tenant
    // Apply.
    let applied = matches!(&outcome, Ok(TransitionOutcome::Applied { .. }));
    assert!(
        !applied,
        "[{}] cross-tenant commit must NEVER Apply",
        backend.name()
    );
    // No cross-tenant version oracle: a `VersionConflict` from a
    // cross-scope probe must report `actual: 0` (indistinguishable from a
    // missing row), never echo the victim row's real counter. The victim
    // was created at version 1, so a leak would surface as `actual: 1`.
    if let Ok(TransitionOutcome::VersionConflict { actual }) = &outcome {
        assert_eq!(
            *actual,
            0,
            "[{}] cross-scope conflict leaked the victim's version counter \
             (got actual={actual}); it must be 0",
            backend.name()
        );
    }
}

/// The spec-16 workflow split contract: a workflow row round-trips by id
/// and by slug, a soft-deleted row disappears from reads / `list` /
/// `get_by_slug`, `update` is a strict CAS (stale `expected_version` →
/// `Conflict`, missing row → `NotFound`), a duplicate id → `Duplicate`,
/// and the version store round-trips a version and lists newest-first.
/// Asserted across every backend so the SQL adapters match the in-memory
/// reference exactly.
pub(crate) async fn assert_workflow_store_contract(backend: &dyn Backend) {
    let wf = backend.workflow_store().await;
    let ver = backend.workflow_version_store().await;
    let s = scope_a();

    let rec = WorkflowRecord {
        id: "wf_c".into(),
        scope: s.clone(),
        version: 0,
        slug: "billing".into(),
        deleted: false,
    };
    wf.create(&s, rec.clone()).await.expect("create");

    // Duplicate id is a Duplicate, not a silent overwrite.
    let dup = wf.create(&s, rec.clone()).await;
    assert!(
        matches!(dup, Err(StorageError::Duplicate { .. })),
        "[{}] duplicate workflow id must be Duplicate, got {dup:?}",
        backend.name()
    );

    // Round-trip by id and by slug.
    let by_id = wf
        .get(&s, "wf_c")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] workflow row by id", backend.name()));
    assert_eq!(by_id.slug, "billing");
    let by_slug = wf
        .get_by_slug(&s, "billing")
        .await
        .expect("get_by_slug")
        .unwrap_or_else(|| panic!("[{}] workflow row by slug", backend.name()));
    assert_eq!(by_slug.id, "wf_c");

    // CAS update: stale expected_version is rejected.
    let stale = wf
        .update(
            &s,
            WorkflowRecord {
                version: 1,
                ..by_id.clone()
            },
            999,
        )
        .await;
    assert!(
        matches!(stale, Err(StorageError::Conflict { .. })),
        "[{}] stale CAS update must Conflict, got {stale:?}",
        backend.name()
    );
    // CAS update with the right expected_version succeeds.
    wf.update(
        &s,
        WorkflowRecord {
            version: 1,
            slug: "billing-v2".into(),
            ..by_id.clone()
        },
        0,
    )
    .await
    .expect("CAS update at expected_version 0");
    let updated = wf
        .get(&s, "wf_c")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] updated row", backend.name()));
    assert_eq!(updated.version, 1);
    assert_eq!(updated.slug, "billing-v2");

    // Update of a missing row is NotFound (never an implicit insert).
    let missing = wf
        .update(
            &s,
            WorkflowRecord {
                id: "wf_absent".into(),
                ..by_id.clone()
            },
            0,
        )
        .await;
    assert!(
        matches!(missing, Err(StorageError::NotFound { .. })),
        "[{}] update of a missing row must be NotFound, got {missing:?}",
        backend.name()
    );

    // Soft-delete removes the row from reads, slug lookup, and list.
    wf.soft_delete(&s, "wf_c").await.expect("soft_delete");
    assert!(
        wf.get(&s, "wf_c").await.expect("get").is_none(),
        "[{}] soft-deleted row must be a read miss",
        backend.name()
    );
    assert!(
        wf.get_by_slug(&s, "billing-v2")
            .await
            .expect("get_by_slug")
            .is_none(),
        "[{}] soft-deleted row must not resolve by slug",
        backend.name()
    );
    assert!(
        wf.list(&s)
            .await
            .expect("list")
            .iter()
            .all(|r| r.id != "wf_c"),
        "[{}] soft-deleted row must not appear in list",
        backend.name()
    );
    // A soft-deleted row is invisible to `update` too: updating a
    // tombstone must be `NotFound` (matching `get`), never resurrect the
    // row and never report a spurious `Conflict`. Without the
    // `deleted = FALSE` / `!deleted` guard on every backend's `update`
    // this would rewrite the tombstone back into a live row.
    let revive = wf
        .update(
            &s,
            WorkflowRecord {
                id: "wf_c".into(),
                deleted: false,
                version: 2,
                ..by_id.clone()
            },
            1,
        )
        .await;
    assert!(
        matches!(revive, Err(StorageError::NotFound { .. })),
        "[{}] update of a soft-deleted row must be NotFound (no revival, \
         no spurious Conflict), got {revive:?}",
        backend.name()
    );
    assert!(
        wf.get(&s, "wf_c").await.expect("get").is_none(),
        "[{}] soft-deleted row must stay a read miss after a rejected \
         update",
        backend.name()
    );

    // Soft-deleting an absent row is NotFound.
    let del_missing = wf.soft_delete(&s, "wf_absent").await;
    assert!(
        matches!(del_missing, Err(StorageError::NotFound { .. })),
        "[{}] soft-delete of a missing row must be NotFound, got {del_missing:?}",
        backend.name()
    );

    // Version store: create + round-trip + duplicate guard + list order.
    for n in 1u32..=3 {
        ver.create(
            &s,
            WorkflowVersionRecord {
                activation: None,
                workflow_id: "wf_v".into(),
                number: n,
                published: false,
                pinned: false,
                definition: serde_json::json!({ "n": n }),
            },
        )
        .await
        .expect("version create");
    }
    let dup_ver = ver
        .create(
            &s,
            WorkflowVersionRecord {
                activation: None,
                workflow_id: "wf_v".into(),
                number: 2,
                published: false,
                pinned: false,
                definition: serde_json::json!({}),
            },
        )
        .await;
    assert!(
        matches!(dup_ver, Err(StorageError::Duplicate { .. })),
        "[{}] duplicate (workflow,number) must be Duplicate, got {dup_ver:?}",
        backend.name()
    );
    let got_v2 = ver
        .get(&s, "wf_v", 2)
        .await
        .expect("version get")
        .unwrap_or_else(|| panic!("[{}] version 2", backend.name()));
    assert_eq!(got_v2.definition, serde_json::json!({ "n": 2 }));
    let listed: Vec<u32> = ver
        .list(&s, "wf_v")
        .await
        .expect("version list")
        .iter()
        .map(|r| r.number)
        .collect();
    assert_eq!(
        listed,
        vec![3, 2, 1],
        "[{}] version list must be newest-first",
        backend.name()
    );
}

/// `WorkflowStore::save_with_published_version` is a real all-or-nothing
/// unit of work on every backend: the row write and the published-version
/// write either both land or neither does. This locks the spec-16
/// orphan-row invariant (a workflow row with no published version is
/// invisible to readers — "the workflow vanished") that the previous
/// two-await sequence could violate on a partial failure.
pub(crate) async fn assert_save_with_published_version_is_atomic(
    backend: &dyn Backend,
) -> serde_json::Value {
    let wf = backend.workflow_store().await;
    let ver = backend.workflow_version_store().await;
    let s = scope_a();

    // 1. Create commits BOTH the row and version #1 as one unit.
    wf.save_with_published_version(
        &s,
        WorkflowRecord {
            id: "wf_atomic".into(),
            scope: s.clone(),
            version: 1,
            slug: "wf_atomic".into(),
            deleted: false,
        },
        WorkflowVersionRecord {
            activation: None,
            workflow_id: "wf_atomic".into(),
            number: 1,
            published: true,
            pinned: false,
            definition: serde_json::json!({ "v": 1 }),
        },
        None,
    )
    .await
    .expect("atomic create");
    assert!(
        wf.get(&s, "wf_atomic").await.expect("get").is_some(),
        "[{}] row must exist after atomic create",
        backend.name()
    );
    let pub1 = ver
        .get_published(&s, "wf_atomic")
        .await
        .expect("get_published")
        .unwrap_or_else(|| panic!("[{}] published version after create", backend.name()));
    assert_eq!(
        pub1.number,
        1,
        "[{}] published version #1 must exist after atomic create",
        backend.name()
    );

    // 2. A CAS update at the WRONG expected_version must roll BOTH back:
    //    the row counter must NOT advance and version #2 must NOT appear.
    let conflict = wf
        .save_with_published_version(
            &s,
            WorkflowRecord {
                id: "wf_atomic".into(),
                scope: s.clone(),
                version: 2,
                slug: "wf_atomic".into(),
                deleted: false,
            },
            WorkflowVersionRecord {
                activation: None,
                workflow_id: "wf_atomic".into(),
                number: 2,
                published: true,
                pinned: false,
                definition: serde_json::json!({ "v": 2 }),
            },
            Some(999), // stale expected version
        )
        .await;
    assert!(
        matches!(conflict, Err(StorageError::Conflict { .. })),
        "[{}] stale-CAS atomic save must Conflict, got {conflict:?}",
        backend.name()
    );
    let row_after = wf
        .get(&s, "wf_atomic")
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] row still present", backend.name()));
    assert_eq!(
        row_after.version,
        1,
        "[{}] row counter must NOT advance on a rolled-back atomic save",
        backend.name()
    );
    assert!(
        ver.get(&s, "wf_atomic", 2)
            .await
            .expect("version get")
            .is_none(),
        "[{}] version #2 must NOT exist after a rolled-back atomic save \
         (no orphan version; the unit rolled back whole)",
        backend.name()
    );

    // 3. A create whose version slot is already taken must roll the row
    //    back too (the row insert must not survive the version failure).
    let dup = wf
        .save_with_published_version(
            &s,
            WorkflowRecord {
                id: "wf_atomic2".into(),
                scope: s.clone(),
                version: 1,
                slug: "wf_atomic2".into(),
                deleted: false,
            },
            WorkflowVersionRecord {
                activation: None,
                // Collides with wf_atomic's existing version #1.
                workflow_id: "wf_atomic".into(),
                number: 1,
                published: true,
                pinned: false,
                definition: serde_json::json!({ "dup": true }),
            },
            None,
        )
        .await;
    assert!(
        matches!(dup, Err(StorageError::Duplicate { .. })),
        "[{}] atomic create with a taken version slot must Duplicate, got {dup:?}",
        backend.name()
    );
    assert!(
        wf.get(&s, "wf_atomic2").await.expect("get").is_none(),
        "[{}] the new row must NOT survive a failed atomic create \
         (row insert rolled back with the version failure)",
        backend.name()
    );
    serde_json::json!({
        "created_workflow_version": row_after.version,
        "published_version": pub1.number,
        "stale_cas_outcome": "Conflict",
        "row_version_after_stale_cas": row_after.version,
        "candidate_version_after_stale_cas": null,
        "duplicate_version_outcome": "Duplicate",
        "orphan_workflow_after_duplicate": false
    })
}

/// `get_published` returns the **highest-numbered** published version when
/// more than one row is marked published (a stale publish that was never
/// cleared). The original in-memory `find` returned an arbitrary
/// `HashMap`-order row; this locks the deterministic
/// `ORDER BY number DESC LIMIT 1` contract across every backend.
pub(crate) async fn assert_get_published_is_highest_numbered(backend: &dyn Backend) {
    let ver = backend.workflow_version_store().await;
    let s = scope_a();
    // Two published versions for the same workflow (1 and 3) plus an
    // unpublished one (2) — `get_published` must return version 3.
    for (n, published) in [(1u32, true), (2, false), (3, true)] {
        ver.create(
            &s,
            WorkflowVersionRecord {
                activation: None,
                workflow_id: "wf_pub".into(),
                number: n,
                published,
                pinned: false,
                definition: serde_json::json!({ "v": n }),
            },
        )
        .await
        .expect("version create");
    }
    let published = ver
        .get_published(&s, "wf_pub")
        .await
        .expect("get_published")
        .unwrap_or_else(|| panic!("[{}] a published version exists", backend.name()));
    assert_eq!(
        published.number,
        3,
        "[{}] get_published must return the highest-numbered published \
         version (deterministic), got {}",
        backend.name(),
        published.number
    );
    assert_eq!(published.definition, serde_json::json!({ "v": 3 }));
}

/// A `commit`'s outbox rows are claimable through the control queue, and
/// the claiming processor fences `mark_completed` (a stale runner whose
/// row was reclaimed cannot flip a newer claim). Also exercises the
/// typed-16-byte-id contract end to end.
pub(crate) async fn assert_control_queue_outbox_and_fencing(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let s = scope_a();
    store
        .create(&s, "exe_cq", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(&s, "exe_cq", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease", backend.name()));
    let msg = ControlMsg {
        id: [42u8; 16],
        execution_id: "exe_cq".into(),
        command: ControlCommand::Cancel,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_cq")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "cancelling"}))
        .outbox(vec![msg])
        .build()
        .expect("batch");
    let outcome = store.commit(batch).await.expect("commit");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "[{}] expected Applied, got {outcome:?}",
        backend.name()
    );

    let runner_a = [1u8; 16];
    let claimed = queue
        .claim_pending(&runner_a, 16)
        .await
        .expect("claim_pending");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] the commit's outbox row must be claimable",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.id,
        [42u8; 16],
        "[{}] typed 16-byte id round-trips through the queue",
        backend.name()
    );
    let current = &claimed[0].token;

    let foreign_scope = ControlClaimToken::new([42u8; 16], current.generation(), scope_b());
    let foreign_acknowledgement = queue.mark_completed(&foreign_scope).await;
    assert!(
        matches!(
            foreign_acknowledgement,
            Err(StorageError::NotFound {
                entity: "control_queue",
                ..
            })
        ),
        "[{}] a claim bound to another tenant must receive uniform NotFound, got: {foreign_acknowledgement:?}",
        backend.name()
    );

    // A token naming a generation this row never reached must be rejected.
    // Under the old `processed_by` fence this was expressed as "a different
    // processor id"; authority is now the token, which a caller cannot forge
    // into existence, so the equivalent probe is a wrong generation.
    let forged = ControlClaimToken::new(
        [42u8; 16],
        ClaimGeneration::new(current.generation().get() + 1),
        current.scope().clone(),
    );
    let stale_ack = queue.mark_completed(&forged).await;
    assert!(
        matches!(stale_ack, Err(StorageError::FencedOut { .. })),
        "[{}] an acknowledgement with a non-current generation must be FencedOut, got: {stale_ack:?}",
        backend.name()
    );
    let reclaimed = queue
        .claim_pending(&runner_a, 16)
        .await
        .expect("claim_pending after stale ack");
    assert!(
        reclaimed.is_empty(),
        "[{}] a fenced ack must be a no-op (row stays Processing, \
         not re-Pending and not Completed)",
        backend.name()
    );

    // The holder of the current claim can complete it.
    queue
        .mark_completed(current)
        .await
        .expect("mark_completed (claimant)");
}

/// A `ControlMsg` whose `resume_target` is `Some(ResumeTarget::Webhook{..})`
/// survives an enqueue→claim round-trip through the durable queue intact.
///
/// This is the structural fix for ADR-0099 W-S3a: closing the confused-deputy
/// bug on the durable path requires the column to exist on both enqueue and
/// claim. A `None` target also round-trips correctly (backward compatibility
/// with legacy rows).
///
/// **Falsifiability**: before the `resume_target TEXT` column was added to
/// `port_control_queue`, `claim_pending` hardcoded `resume_target: None` and
/// the `Some(target)` assertion failed → RED.
pub(crate) async fn assert_resume_target_survives_queue_round_trip(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let s = scope_a();
    store
        .create(&s, "exe_rt", "wf_rt", serde_json::json!({}))
        .await
        .expect("create execution for resume-target round-trip");
    let token = store
        .acquire_lease(&s, "exe_rt", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease for resume-target test", backend.name()));

    let webhook_target = ResumeTarget::Webhook {
        callback_id: "cb-round-trip".to_owned(),
    };
    let resume_msg = ControlMsg {
        id: [77u8; 16],
        execution_id: "exe_rt".into(),
        command: ControlCommand::Resume,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: Some(webhook_target.clone()),
    };
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_rt")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "waiting"}))
        .outbox(vec![resume_msg])
        .build()
        .expect("batch for resume-target round-trip");
    store
        .commit(batch)
        .await
        .expect("commit for resume-target round-trip");

    let runner = [9u8; 16];
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim_pending for resume-target round-trip");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] the outbox Resume must be claimable",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.resume_target,
        Some(webhook_target),
        "[{}] resume_target must survive the enqueue→claim round-trip intact \
         (kind + identity both preserved)",
        backend.name()
    );

    // A second message with no target must also round-trip correctly
    // (backward-compatibility: legacy rows and non-Resume commands have NULL).
    store
        .create(&s, "exe_rt2", "wf_rt", serde_json::json!({}))
        .await
        .expect("create second execution");
    let token2 = store
        .acquire_lease(&s, "exe_rt2", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease 2")
        .unwrap_or_else(|| panic!("[{}] lease 2 for resume-target test", backend.name()));
    let null_msg = ControlMsg {
        id: [78u8; 16],
        execution_id: "exe_rt2".into(),
        command: ControlCommand::Cancel,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let batch2 = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_rt2")
        .expected_version(0)
        .fencing(token2)
        .new_state(serde_json::json!({"s": "cancelling"}))
        .outbox(vec![null_msg])
        .build()
        .expect("batch 2");
    store.commit(batch2).await.expect("commit 2");
    // First, drain the already-claimed row above to avoid re-claiming it.
    queue
        .mark_completed(&claimed[0].token)
        .await
        .expect("mark_completed first row");
    let claimed2 = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim_pending 2");
    assert_eq!(
        claimed2.len(),
        1,
        "[{}] the null-target message must be claimable",
        backend.name()
    );
    assert_eq!(
        claimed2[0].msg.resume_target,
        None,
        "[{}] a None resume_target must round-trip as None (legacy compat)",
        backend.name()
    );
}

/// Enqueue a single control row of `command` and drive its `reclaim_count`
/// up to `target_count` by repeated claim → reclaim cycles, leaving it
/// `Processing` (just-claimed) at `reclaim_count == target_count`.
///
/// Each cycle claims the (Pending) row, then sweeps with a budget above the
/// current count so the REDELIVER branch fires (`Processing → Pending`,
/// `reclaim_count += 1`). The cutoff is wall-clock (`chrono`), so — like the
/// `refresh_claim_*` reclaim tests for this same layer — a short real sleep
/// makes the just-claimed row reliably stale; `tokio::time::pause` cannot drive
/// the SQL backends' `chrono::Utc::now()` cutoff. Returns the row id so the
/// caller can keep claiming it.
async fn enqueue_and_climb_reclaim_count(
    backend: &dyn Backend,
    command: ControlCommand,
    target_count: u32,
) -> [u8; 16] {
    let queue = backend.control_queue().await;
    let s = scope_a();
    let row_id = [
        0xC1,
        0xA1,
        0x33,
        0x44,
        0x55,
        0x66,
        0x77,
        0x88,
        command as u8,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    let msg = ControlMsg {
        id: row_id,
        execution_id: "exe_reclaim".into(),
        command,
        scope: s.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    queue.enqueue(&msg).await.expect("enqueue reclaim row");

    let runner = [0xCC_u8; 16];
    for cycle in 0..target_count {
        let claimed = queue
            .claim_pending(&runner, 16)
            .await
            .expect("claim during climb");
        assert!(
            claimed.iter().any(|c| c.msg.id == row_id),
            "[{}] the reclaim row must be claimable on climb cycle {cycle}",
            backend.name()
        );
        // Make the just-claimed row stale against the wall-clock cutoff.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        // Budget strictly above the current count so this row REDELIVERS
        // (Processing → Pending, reclaim_count += 1) rather than exhausting.
        queue
            .reclaim_stuck(std::time::Duration::ZERO, target_count + 1)
            .await
            .expect("reclaim during climb");
    }
    // Re-claim so the assertion sweep sees it `Processing` at the target count.
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("final claim at target count");
    let row = claimed
        .iter()
        .find(|c| c.msg.id == row_id)
        .unwrap_or_else(|| panic!("[{}] reclaim row must be re-claimable", backend.name()));
    assert_eq!(
        row.msg.reclaim_count,
        target_count,
        "[{}] the row must reach reclaim_count == {target_count} before the assertion sweep",
        backend.name()
    );
    row_id
}

/// **ADR-0099 W-S3b** — a `command = 'Resume'` row at `reclaim_count == max`,
/// past `reclaim_after`, is EXEMPT from the reclaim budget: the exhaust sweep
/// must NOT Fail it; it stays redeliverable (`Processing → Pending`) so a later
/// claim still delivers it. Engine liveness + the wait's own timeout are the
/// only terminal authorities for a parked Resume.
///
/// Observable through the trait alone: after the assertion sweep, a fresh
/// `claim_pending` still returns the Resume row (a Failed row is terminal and
/// would never be claimable again).
///
/// **Falsifiability**: revert the `command <> 'Resume'` exhaust guard (and its
/// `OR command = 'Resume'` redeliver complement) → the row at `reclaim_count >=
/// max` is force-Failed → the post-sweep `claim_pending` finds nothing → the
/// "still claimable" assertion fails → RED.
pub(crate) async fn assert_resume_row_exempt_from_reclaim_budget(backend: &dyn Backend) {
    let max_reclaim_count = 2;
    let row_id =
        enqueue_and_climb_reclaim_count(backend, ControlCommand::Resume, max_reclaim_count).await;
    let queue = backend.control_queue().await;

    // The assertion sweep: the Resume row is now `Processing` at
    // `reclaim_count == max`, past `reclaim_after`. The budget would normally
    // exhaust it (→ Failed); the exemption must redeliver it instead.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let outcome = queue
        .reclaim_stuck(std::time::Duration::ZERO, max_reclaim_count)
        .await
        .expect("assertion reclaim sweep");
    assert_eq!(
        outcome.exhausted,
        0,
        "[{}] a Resume row at reclaim_count == max must NOT be exhausted (Failed); \
         it is budget-exempt",
        backend.name()
    );

    // The exempt Resume stays redeliverable: a fresh claim still delivers it.
    let runner = [0xAB; 16];
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim after the assertion sweep");
    let row = claimed.iter().find(|c| c.msg.id == row_id);
    assert!(
        row.is_some(),
        "[{}] a budget-exempt Resume row must stay redeliverable after the exhaust \
         sweep (still claimable), not be Failed",
        backend.name()
    );
    assert!(
        row.is_some_and(|c| c.msg.reclaim_count > max_reclaim_count),
        "[{}] the redelivered Resume row's reclaim_count must keep climbing past max \
         (observable stuck-Resume signal), got {:?}",
        backend.name(),
        row.map(|c| c.msg.reclaim_count)
    );
}

/// **ADR-0099 W-S3b** — the exemption is Resume-ONLY: a non-Resume row
/// (`Start` / `Cancel`) at `reclaim_count >= max`, past `reclaim_after`, still
/// EXHAUSTS to `Failed` (the budget remains the terminal authority for rows that
/// do real work and can poison-loop).
///
/// Observable through the trait alone: after the assertion sweep, a fresh
/// `claim_pending` finds nothing (a Failed row is terminal, never re-claimable).
///
/// **Falsifiability**: widen the exemption to cover non-Resume commands → the
/// `Start` row at `reclaim_count >= max` redelivers instead of failing → the
/// post-sweep `claim_pending` returns it → the "must be Failed (not claimable)"
/// assertion fails → RED.
pub(crate) async fn assert_non_resume_row_still_exhausts(backend: &dyn Backend) {
    let max_reclaim_count = 2;
    let row_id =
        enqueue_and_climb_reclaim_count(backend, ControlCommand::Start, max_reclaim_count).await;
    let queue = backend.control_queue().await;

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let outcome = queue
        .reclaim_stuck(std::time::Duration::ZERO, max_reclaim_count)
        .await
        .expect("assertion reclaim sweep");
    assert_eq!(
        outcome.exhausted,
        1,
        "[{}] a non-Resume row at reclaim_count >= max must be exhausted (Failed) — \
         the exemption is Resume-only",
        backend.name()
    );

    // A Failed row is terminal: it must NOT be claimable again.
    let runner = [0xCD; 16];
    let claimed = queue
        .claim_pending(&runner, 16)
        .await
        .expect("claim after the assertion sweep");
    assert!(
        claimed.iter().all(|c| c.msg.id != row_id),
        "[{}] an exhausted non-Resume row must be Failed (terminal), never re-claimable",
        backend.name()
    );
}

/// Journal entries appended by a `commit` are readable in order, and a
/// cross-tenant read yields an empty journal (never another tenant's
/// entries).
pub(crate) async fn assert_journal_visibility_and_scope(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let reader = backend.journal_reader().await;
    let s = scope_a();
    store
        .create(&s, "exe_j", "wf_1", serde_json::json!({}))
        .await
        .expect("create");
    let token = store
        .acquire_lease(&s, "exe_j", "holder", std::time::Duration::from_secs(30))
        .await
        .expect("acquire_lease")
        .unwrap_or_else(|| panic!("[{}] lease", backend.name()));
    let batch = TransitionBatch::builder()
        .scope(s.clone())
        .execution_id("exe_j")
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"s": "running"}))
        .journal(vec![
            JournalEntry {
                seq: None,
                payload: serde_json::json!({"e": "a"}),
            },
            JournalEntry {
                seq: None,
                payload: serde_json::json!({"e": "b"}),
            },
        ])
        .build()
        .expect("batch");
    store.commit(batch).await.expect("commit");

    let entries = reader.get_journal(&s, "exe_j").await.expect("get_journal");
    assert_eq!(
        entries.len(),
        2,
        "[{}] both journal entries must be readable",
        backend.name()
    );
    assert_eq!(
        entries[0].payload,
        serde_json::json!({"e": "a"}),
        "[{}] journal entries must be ordered oldest-first",
        backend.name()
    );

    // Cross-tenant read: never another tenant's journal.
    let cross = reader
        .get_journal(&scope_b(), "exe_j")
        .await
        .expect("get_journal cross-scope");
    assert!(
        cross.is_empty(),
        "[{}] a cross-tenant journal read must be empty",
        backend.name()
    );
}

/// The durable idempotent-replay cache is first-writer-wins: a second
/// `put` on the same key keeps the original record + fingerprint (replay
/// race). Purely within `scope_a`, so it is decorator-transparent and runs
/// in both the raw and scoped matrices.
pub(crate) async fn assert_idempotency_store_first_writer(backend: &dyn Backend) {
    let store = backend.idempotency_store().await;
    let raw_key = "POST /x:idem-1".to_string();
    let first = CachedRecord {
        status: 200,
        headers: b"h1".to_vec(),
        body: b"first".to_vec(),
        fingerprint: b"fp-first".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    };
    let second = CachedRecord {
        status: 500,
        headers: b"h2".to_vec(),
        body: b"second".to_vec(),
        fingerprint: b"fp-second".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    };
    store
        .put(
            &scope_a(),
            raw_key.clone(),
            first.clone(),
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put #1");
    store
        .put(
            &scope_a(),
            raw_key.clone(),
            second,
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put #2 (must be a no-op)");
    let got = store
        .get(&scope_a(), &raw_key)
        .await
        .expect("get")
        .unwrap_or_else(|| panic!("[{}] cached record must be present", backend.name()));
    assert_eq!(
        got.body,
        b"first",
        "[{}] first-writer-wins: the original body must survive a replay race",
        backend.name()
    );
    assert_eq!(
        got.fingerprint,
        b"fp-first",
        "[{}] the original fingerprint must survive (replay-mismatch detection)",
        backend.name()
    );
}

/// Tenant isolation of the durable replay cache: the store folds the scope
/// into the stored key, so the *same raw key* under a different scope is a
/// clean miss — tenant A can neither read nor poison tenant B's entry
/// (replay-oracle mitigation, §6.1).
///
/// This passes an explicit foreign scope to probe the adapter's raw
/// scope-fold, so — like the other `cross_scope_*` assertions — it runs
/// only in the raw matrix. The decorator substitutes the per-call scope
/// away by design, so decorator-level cross-tenant denial is proven in
/// `cross_tenant_denial.rs` instead.
pub(crate) async fn assert_idempotency_store_cross_scope_isolated(backend: &dyn Backend) {
    let store = backend.idempotency_store().await;
    let raw_key = "POST /x:idem-1".to_string();
    let record = CachedRecord {
        status: 200,
        headers: b"h1".to_vec(),
        body: b"a-only".to_vec(),
        fingerprint: b"fp-a".to_vec(),
        expires_at: "2999-01-01T00:00:00Z".into(),
    };
    store
        .put(
            &scope_a(),
            raw_key.clone(),
            record,
            std::time::Duration::from_mins(1),
        )
        .await
        .expect("put under scope A");

    // A different tenant probing the *same raw key* is a clean miss — the
    // store-side scope fold makes it a different stored key, never tenant
    // A's record.
    let cross = store
        .get(&scope_b(), &raw_key)
        .await
        .expect("get cross-scope key");
    assert!(
        cross.is_none(),
        "[{}] a cross-tenant cache key must not resolve to another tenant's record",
        backend.name()
    );
}

/// Webhook activation upsert → resolve → deactivate, with tenant
/// isolation: the same slug in a different tenant does not resolve, and a
/// deactivated activation stops routing.
///
/// Also covers the ADR-0096 extended fields: safe-default round-trip
/// (new fields default to `Test` / `None` / zero-sentinel) and full
/// round-trip of `workflow_id`, `mode`, and `token_hash`.
pub(crate) async fn assert_webhook_activation_and_scope(backend: &dyn Backend) {
    let store = backend.webhook_store().await;
    let s = scope_a();
    // Use the constructor so the call site is future-proof against further
    // `#[non_exhaustive]` field additions (ADR-0096 commit 1 PREREQ).
    store
        .upsert(
            &s,
            WebhookActivationRecord::new("trg_1", s.clone(), "deploy-hook", true),
        )
        .await
        .expect("upsert");

    let resolved = store
        .resolve(&s, "deploy-hook")
        .await
        .expect("resolve")
        .unwrap_or_else(|| panic!("[{}] active activation must resolve", backend.name()));
    assert_eq!(
        resolved.trigger_id,
        "trg_1",
        "[{}] resolve returns the owning trigger",
        backend.name()
    );
    // Safe-default proof: a row inserted without an explicit mode must
    // resolve with `Test`.  If the schema default were `'prod'` this
    // assertion would fail — proving the migration default is load-bearing.
    assert_eq!(
        resolved.mode,
        WebhookMode::Test,
        "[{}] default mode must be Test (safe-default invariant)",
        backend.name()
    );

    // Same slug, different tenant → miss (slug is unique per tenant; a
    // webhook never crosses a tenant boundary).
    let cross = store
        .resolve(&scope_b(), "deploy-hook")
        .await
        .expect("resolve cross-scope");
    assert!(
        cross.is_none(),
        "[{}] a slug must not resolve across a tenant boundary",
        backend.name()
    );

    // Deactivation stops routing (never dispatch a paused webhook).
    store.deactivate(&s, "trg_1").await.expect("deactivate");
    let after = store
        .resolve(&s, "deploy-hook")
        .await
        .expect("resolve after deactivate");
    assert!(
        after.is_none(),
        "[{}] a deactivated activation must not resolve",
        backend.name()
    );

    // ── Extended-fields round-trip ────────────────────────────────────────
    // Upsert a record with all three ADR-0096 fields set to non-default
    // values and verify exact round-trip (no tautological `is_some()`).
    let token = [0xde_u8; 32];
    let mut extended = WebhookActivationRecord::new("trg_2", s.clone(), "prod-hook", true);
    extended.workflow_id = Some("wf_abc".to_string());
    extended.mode = WebhookMode::Prod;
    extended.token_hash = token;
    store.upsert(&s, extended).await.expect("upsert extended");

    let got = store
        .resolve(&s, "prod-hook")
        .await
        .expect("resolve extended")
        .unwrap_or_else(|| panic!("[{}] extended activation must resolve", backend.name()));
    assert_eq!(
        got.workflow_id.as_deref(),
        Some("wf_abc"),
        "[{}] workflow_id must round-trip exactly",
        backend.name()
    );
    assert_eq!(
        got.mode,
        WebhookMode::Prod,
        "[{}] mode must round-trip exactly",
        backend.name()
    );
    assert_eq!(
        got.token_hash,
        token,
        "[{}] token_hash must round-trip exactly",
        backend.name()
    );

    // Cross-tenant isolation is carried forward: `prod-hook` in scope_b
    // must not resolve, even though scope_a has an active activation for it.
    let cross_ext = store
        .resolve(&scope_b(), "prod-hook")
        .await
        .expect("resolve cross-scope extended");
    assert!(
        cross_ext.is_none(),
        "[{}] extended activation must not resolve across a tenant boundary",
        backend.name()
    );
}

/// System-surface methods: `resolve_by_token` + `list_all_active`.
///
/// Proves:
/// - Single-row token resolution with exact-value asserts (no tautological
///   `is_some()`).
/// - Cross-tenant isolation: resolving tenant A's hash never yields tenant B's
///   row.
/// - Sentinel rejection: the all-zeros token hash always returns `None`
///   without querying.
/// - Unknown hash returns `None` (no false-positive).
/// - `list_all_active` enumerates rows from both tenants (cross-tenant
///   bootstrap enumeration).
pub(crate) async fn assert_webhook_system_surface(backend: &dyn Backend) {
    let store = backend.webhook_store().await;
    let sa = scope_a();
    let sb = scope_b();

    // Upsert two rows under different scopes, each with a distinct token hash
    // and workflow_id so exact-value asserts are meaningful.
    let hash_a: [u8; 32] = [0xa1; 32];
    let hash_b: [u8; 32] = [0xb2; 32];

    let mut row_a = WebhookActivationRecord::new("trg_sys_a", sa.clone(), "sys-hook-a", true);
    row_a.workflow_id = Some("wf_a".to_string());
    row_a.token_hash = hash_a;
    store.upsert(&sa, row_a).await.expect("upsert row_a");

    let mut row_b = WebhookActivationRecord::new("trg_sys_b", sb.clone(), "sys-hook-b", true);
    row_b.workflow_id = Some("wf_b".to_string());
    row_b.token_hash = hash_b;
    store.upsert(&sb, row_b).await.expect("upsert row_b");

    // ── resolve_by_token: tenant A's hash → A's row ───────────────────────
    let got_a = store
        .resolve_by_token(&hash_a)
        .await
        .expect("resolve_by_token hash_a")
        .unwrap_or_else(|| {
            panic!(
                "[{}] resolve_by_token(hash_a) must return Some",
                backend.name()
            )
        });
    assert_eq!(
        got_a.trigger_id,
        "trg_sys_a",
        "[{}] resolve_by_token(hash_a) must return row_a's trigger_id",
        backend.name()
    );
    assert_eq!(
        got_a.scope,
        sa,
        "[{}] resolve_by_token(hash_a) must carry scope_a",
        backend.name()
    );
    assert_eq!(
        got_a.workflow_id.as_deref(),
        Some("wf_a"),
        "[{}] resolve_by_token(hash_a) must carry wf_a",
        backend.name()
    );
    assert_eq!(
        got_a.token_hash,
        hash_a,
        "[{}] resolve_by_token(hash_a) must round-trip token_hash",
        backend.name()
    );

    // ── resolve_by_token: tenant B's hash → B's row ───────────────────────
    let got_b = store
        .resolve_by_token(&hash_b)
        .await
        .expect("resolve_by_token hash_b")
        .unwrap_or_else(|| {
            panic!(
                "[{}] resolve_by_token(hash_b) must return Some",
                backend.name()
            )
        });
    assert_eq!(
        got_b.trigger_id,
        "trg_sys_b",
        "[{}] resolve_by_token(hash_b) must return row_b's trigger_id",
        backend.name()
    );
    assert_eq!(
        got_b.scope,
        sb,
        "[{}] resolve_by_token(hash_b) must carry scope_b",
        backend.name()
    );
    assert_eq!(
        got_b.workflow_id.as_deref(),
        Some("wf_b"),
        "[{}] resolve_by_token(hash_b) must carry wf_b",
        backend.name()
    );

    // Cross-tenant isolation: A's hash must never yield B's row.
    assert_ne!(
        got_a.trigger_id,
        got_b.trigger_id,
        "[{}] resolve_by_token must never cross-pollinate tenant rows",
        backend.name()
    );

    // ── Sentinel rejection: [0u8;32] → None (no query) ───────────────────
    let sentinel = store
        .resolve_by_token(&[0u8; 32])
        .await
        .expect("resolve_by_token sentinel");
    assert!(
        sentinel.is_none(),
        "[{}] the all-zeros sentinel must always return None",
        backend.name()
    );

    // ── Unknown hash → None ───────────────────────────────────────────────
    let unknown = store
        .resolve_by_token(&[0xff; 32])
        .await
        .expect("resolve_by_token unknown");
    assert!(
        unknown.is_none(),
        "[{}] an unknown hash must return None",
        backend.name()
    );

    // ── Deactivated row must not resolve by token (F1) ────────────────────
    //
    // Deactivate A's row; `resolve_by_token` must return `None` even though
    // the token_hash is still stored.  This guards the `AND active = TRUE`
    // predicate across all three backends.
    store
        .deactivate(&sa, "trg_sys_a")
        .await
        .expect("deactivate trg_sys_a");
    let deactivated = store
        .resolve_by_token(&hash_a)
        .await
        .expect("resolve_by_token after deactivate");
    assert!(
        deactivated.is_none(),
        "[{}] resolve_by_token must return None for a deactivated row",
        backend.name()
    );

    // ── list_all_active: cross-tenant enumeration ─────────────────────────
    let all = store.list_all_active().await.expect("list_all_active");
    let ids: Vec<&str> = all.iter().map(|r| r.trigger_id.as_str()).collect();
    // Row A was deactivated above; only row B must appear.
    assert!(
        !ids.contains(&"trg_sys_a"),
        "[{}] list_all_active must NOT contain deactivated trg_sys_a",
        backend.name()
    );
    assert!(
        ids.contains(&"trg_sys_b"),
        "[{}] list_all_active must contain trg_sys_b (tenant B row)",
        backend.name()
    );
}

/// A [`Backend`] whose stores are wrapped in the `nebula-tenancy`
/// scope-enforcing decorators, all bound to one tenant ([`scope_a`]).
///
/// Run against the **same-tenant** subset of the contract suite this
/// proves the decorator is *transparent* for in-tenant operations: every
/// assertion that operates purely within `scope_a` must stay green when
/// every call goes through the decorator (the substituted bound scope
/// equals the scope the assertion already uses, so it is a no-op there).
///
/// Cross-tenant *denial* — the part the decorator actually adds — is the
/// security property and is proven directly in
/// `tests/cross_tenant_denial.rs` (two decorators, tenants A and B, over
/// one shared adapter). It is intentionally **not** asserted here: the
/// raw `cross_scope_*` / journal / webhook assertions probe the adapter's
/// own `WHERE` filtering with an explicit foreign-scope argument, which
/// the decorator *substitutes away* — a different mechanism, tested in
/// its own suite.
pub(crate) struct ScopedBackend<B: Backend> {
    inner: B,
}

impl<B: Backend + Default> Default for ScopedBackend<B> {
    fn default() -> Self {
        Self {
            inner: B::default(),
        }
    }
}

#[async_trait::async_trait]
impl<B: Backend> Backend for ScopedBackend<B> {
    fn name(&self) -> &'static str {
        // Verbatim inner name so `skip_reason` keeps gating the scoped
        // SQLite/Postgres cases by feature/DATABASE_URL.
        self.inner.name()
    }

    async fn execution_store(&self) -> Arc<dyn ExecutionStore> {
        Arc::new(nebula_tenancy::ScopedExecutionStore::new(
            self.inner.execution_store().await,
            scope_a(),
        ))
    }

    async fn idempotency_guard(&self) -> Arc<dyn IdempotencyGuard> {
        Arc::new(nebula_tenancy::ScopedIdempotencyGuard::new(
            self.inner.idempotency_guard().await,
            scope_a(),
        ))
    }

    async fn control_queue(&self) -> Arc<dyn ControlQueue> {
        Arc::new(nebula_tenancy::ScopedControlQueue::new(
            self.inner.control_queue().await,
            scope_a(),
        ))
    }

    async fn journal_reader(&self) -> Arc<dyn ExecutionJournalReader> {
        Arc::new(nebula_tenancy::ScopedExecutionJournalReader::new(
            self.inner.journal_reader().await,
            scope_a(),
        ))
    }

    async fn idempotency_store(&self) -> Arc<dyn IdempotencyStore> {
        Arc::new(nebula_tenancy::ScopedIdempotencyStore::new(
            self.inner.idempotency_store().await,
            scope_a(),
        ))
    }

    async fn webhook_store(&self) -> Arc<dyn WebhookActivationStore> {
        Arc::new(nebula_tenancy::ScopedWebhookActivationStore::new(
            self.inner.webhook_store().await,
            scope_a(),
        ))
    }

    async fn workflow_store(&self) -> Arc<dyn WorkflowStore> {
        Arc::new(nebula_tenancy::ScopedWorkflowStore::new(
            self.inner.workflow_store().await,
            scope_a(),
        ))
    }

    async fn workflow_version_store(&self) -> Arc<dyn WorkflowVersionStore> {
        Arc::new(nebula_tenancy::ScopedWorkflowVersionStore::new(
            self.inner.workflow_version_store().await,
            scope_a(),
        ))
    }

    // Job dispatch is a worker-global discovery port. Tenant predicates are
    // carried by the storage-minted claim token on acknowledgement.
    async fn job_dispatch_queue(&self) -> Arc<dyn JobDispatchQueue> {
        self.inner.job_dispatch_queue().await
    }

    // Start acceptance is not wrapped by the tenancy decorator: the scope is
    // is already an explicit field of the materialization, so no ambient scope
    // for a decorator to substitute.
    async fn start_acceptance_store(&self) -> Arc<dyn StartAcceptanceStore> {
        self.inner.start_acceptance_store().await
    }

    // The exact catalog is not wrapped either; it is backend-owned.
    async fn plan_flavor_catalog_writer(&self) -> Arc<dyn PlanFlavorCatalogWriter> {
        self.inner.plan_flavor_catalog_writer().await
    }

    async fn plan_flavor_catalog_admin(&self) -> Arc<dyn PlanFlavorCatalogAdmin> {
        self.inner.plan_flavor_catalog_admin().await
    }
}

/// A stable processor identity cannot acknowledge a control command whose
/// claim a reclaim already superseded — the same-processor ABA, on the queue
/// that carries accepted lifecycle commands.
///
/// The `JobDispatchQueue` twin of this assertion is
/// `assert_job_dispatch_same_processor_aba_is_fenced`; both queues had the same
/// `processed_by` fence and therefore the same defect. Here the stakes are a
/// Cancel or Terminate being marked completed by a consumer that no longer owns
/// it, while the attempt that does own it is still dispatching.
pub(crate) async fn assert_control_queue_same_processor_aba_is_fenced(
    backend: &dyn Backend,
) -> serde_json::Value {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let scope = scope_a();
    // One identity for both attempts — that is the whole point.
    let processor = [11u8; 16];

    let execution_id = "exe_control_aba";
    store
        .create(
            &scope,
            execution_id,
            "wf_control_aba",
            serde_json::json!({}),
        )
        .await
        .expect("create the execution the command targets");
    queue
        .enqueue(&ControlMsg {
            id: [0x5Au8; 16],
            execution_id: execution_id.to_owned(),
            command: ControlCommand::Cancel,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        })
        .await
        .expect("enqueue the control command");

    let first = queue
        .claim_pending(&processor, 16)
        .await
        .expect("first claim");
    assert_eq!(
        first.len(),
        1,
        "[{}] generation N must claim the command",
        backend.name()
    );
    let superseded = first[0].token.clone();

    tokio::time::sleep(ABA_CLAIM_AGE).await;
    let outcome = queue
        .reclaim_stuck(ABA_RECLAIM_HORIZON, 16)
        .await
        .expect("reclaim");
    assert_eq!(
        (outcome.reclaimed, outcome.exhausted),
        (1, 0),
        "[{}] the aged claim must be reclaimed, not exhausted",
        backend.name()
    );

    let second = queue
        .claim_pending(&processor, 16)
        .await
        .expect("second claim");
    assert_eq!(
        second.len(),
        1,
        "[{}] generation N+1 must re-claim the command",
        backend.name()
    );
    let current = second[0].token.clone();
    assert!(
        current.generation() > superseded.generation(),
        "[{}] a re-claim must mint a strictly greater generation ({} then {})",
        backend.name(),
        superseded.generation(),
        current.generation()
    );

    let late_ack = queue.mark_completed(&superseded).await;
    assert!(
        matches!(late_ack, Err(StorageError::FencedOut { .. })),
        "[{}] a late ack from generation N must be FencedOut, got: {late_ack:?}",
        backend.name()
    );
    let late_nack = queue
        .mark_failed(&superseded, "late generation-N failure")
        .await;
    assert!(
        matches!(late_nack, Err(StorageError::FencedOut { .. })),
        "[{}] a late nack from generation N must be FencedOut, got: {late_nack:?}",
        backend.name()
    );

    // Zero state change: the row is still owned by generation N+1, which can
    // still acknowledge it.
    queue
        .mark_completed(&current)
        .await
        .expect("generation N+1 still owns the row after the fenced acknowledgements");
    serde_json::json!({
        "queue": "control",
        "row_id": current.row_id(),
        "superseded_generation": superseded.generation().get(),
        "current_generation": current.generation().get(),
        "same_processor": true,
        "late_ack_fenced": true,
        "late_nack_fenced": true,
        "stale_generation_mutation_count": 0,
        "current_owner_completed_after_stale_attempts": true
    })
}

// ── execution revision reference fixtures ───────────────────────────────────

fn materialized_plan_id(seed: u8) -> ExecutablePlanRevisionId {
    let mut bytes = [0_u8; 32];
    bytes[0] = seed;
    bytes[1] = 0x50;
    ExecutablePlanRevisionId::from_bytes(bytes)
}

fn materialized_flavor_id(seed: u8) -> WorkerFlavorRevisionId {
    let mut bytes = [0_u8; 32];
    bytes[0] = seed;
    bytes[1] = 0x46;
    WorkerFlavorRevisionId::from_bytes(bytes)
}

fn materialized_pair(seed: u8) -> PlanFlavorRevisionRecord {
    let workflow_id = WorkflowId::from_bytes([seed; 16]);
    let workflow_revision_id = WorkflowVersionId::from_bytes([seed.wrapping_add(1); 16]);
    let plugin_set_id = PluginSetId::from_bytes([seed.wrapping_add(2); 32]);
    let plan_id = materialized_plan_id(seed);
    let flavor_id = materialized_flavor_id(seed);
    let flavor = WorkerFlavorRevisionRecord::v1_json(
        flavor_id,
        RevisionRecordBytes::try_from_vec(format!(r#"{{"flavor":"{seed}"}}"#).into_bytes())
            .expect("materialized flavor record body is non-empty"),
    );
    PlanFlavorRevisionRecord::graph_v1_json(
        plan_id,
        RevisionRecordBytes::try_from_vec(
            serde_json::to_vec(&serde_json::json!({
                "claimed_id": plan_id,
                "workflow_version_id": workflow_revision_id,
                "worker_flavor_revision_id": flavor_id,
                "plugin_set_id": plugin_set_id,
                "manifest": { "workflow_id": workflow_id }
            }))
            .expect("revision fixture must serialize"),
        )
        .expect("materialized plan record body is non-empty"),
        flavor,
    )
}

async fn install_materialized_pair(backend: &dyn Backend, seed: u8) -> PlanFlavorRevisionRecord {
    let writer = backend.plan_flavor_catalog_writer().await;
    let record = materialized_pair(seed);
    assert_eq!(
        writer.insert(&record).await,
        Ok(RevisionInsertOutcome::Inserted),
        "[{}] a fresh materialized pair installs exactly once",
        backend.name()
    );
    record
}

async fn materialize_execution_reference(
    backend: &dyn Backend,
    scope: &Scope,
    execution_id: &str,
    record: &PlanFlavorRevisionRecord,
    seed: u8,
) -> StartMaterialization {
    let workflow_id = WorkflowId::from_bytes([seed; 16]);
    let workflow_revision_id = WorkflowVersionId::from_bytes([seed.wrapping_add(1); 16]);
    let plugin_set_id = PluginSetId::from_bytes([seed.wrapping_add(2); 32]);
    let mut bundle_bytes = [0_u8; 16];
    bundle_bytes[0] = seed;
    let identity = StartContractIdentity::new(
        ExecutionContractBundleId::from_bytes(bundle_bytes),
        record.ids(),
    );
    let bundle = ContractBundleRecord::v1_json(
        identity,
        serde_json::to_vec(&serde_json::json!({
            "bundle_id": identity.bundle_id(),
            "org_id": &scope.org_id,
            "workspace_id": &scope.workspace_id,
            "executable_plan_revision_id": record.ids().plan(),
            "plugin_set_id": plugin_set_id,
            "revisions": {
                "workflow": workflow_revision_id,
                "worker_flavor": record.ids().worker_flavor()
            }
        }))
        .expect("contract fixture must serialize"),
    )
    .expect("contract fixture must be bounded");
    let state = serde_json::json!({
        "execution_id": execution_id,
        "workflow_id": workflow_id,
        "workflow_version_number": 1,
        "executable_plan_revision_id": record.ids().plan(),
        "worker_flavor_revision_id": record.ids().worker_flavor(),
        "status": "created",
        "version": 0,
        "node_states": {},
        "created_at": "2026-09-06T00:00:00Z",
        "updated_at": "2026-09-06T00:00:00Z",
        "started_at": null,
        "completed_at": null,
        "total_output_bytes": 0,
        "total_retries": 0,
        "terminated_by": null,
        "workflow_input": {}
    });
    let command = ControlMsg {
        id: execution_id
            .parse::<ExecutionId>()
            .expect("execution fixture identity must be typed")
            .as_bytes(),
        execution_id: execution_id.to_owned(),
        command: ControlCommand::Start,
        scope: scope.clone(),
        w3c_traceparent: None,
        reclaim_count: 0,
        resume_target: None,
    };
    let workflow_id = workflow_id.to_string();
    backend
        .start_acceptance_store()
        .await
        .materialize_start(&MaterializedStart::new(
            scope,
            None,
            execution_id,
            NewExecution::new(&workflow_id, &state),
            &command,
            &bundle,
        ))
        .await
        .expect("materialize execution reference")
}

// ── terminal reference transition conformance assertions ─────────────────

async fn assert_reference_counts(
    backend: &dyn Backend,
    record: &PlanFlavorRevisionRecord,
    live: u64,
    rollback: u64,
) {
    let admin = backend.plan_flavor_catalog_admin().await;
    let outcome = admin
        .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
            record.ids().plan(),
        ))
        .await;
    match outcome {
        Ok(BeginDrainOutcome::Started(counts) | BeginDrainOutcome::AlreadyDraining(counts)) => {
            assert_eq!(
                counts.live_executions(),
                live,
                "[{}] live reference count after terminal transition",
                backend.name()
            );
            assert_eq!(
                counts.rollback_windows(),
                rollback,
                "[{}] rollback-window count after terminal transition",
                backend.name()
            );
        },
        Err(error) => panic!(
            "[{}] begin_drain failed while counting references: {error:?}",
            backend.name()
        ),
    }
}

/// A terminal commit with `ReleaseLive` releases the materialized execution's
/// live revision reference atomically with the aggregate state change.
pub(crate) async fn assert_terminal_commit_releases_live_reference(backend: &dyn Backend) {
    let executions = backend.execution_store().await;
    let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    let execution_id = ExecutionId::new().to_string();
    let record = install_materialized_pair(backend, 0x70).await;
    let accepted =
        materialize_execution_reference(backend, &scope, &execution_id, &record, 0x70).await;
    assert!(matches!(accepted, StartMaterialization::Accepted { .. }));

    let token = executions
        .acquire_lease(
            &scope,
            &execution_id,
            "terminal-release",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire lease")
        .expect("lease must be available for a fresh execution");

    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id.clone())
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"status": "Completed"}))
        .reference_transition(ExecutionReferenceTransition::ReleaseLive)
        .build()
        .expect("terminal release batch");
    let outcome = executions
        .commit(batch)
        .await
        .expect("terminal commit must apply");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "[{}] terminal release commit must apply, got {outcome:?}",
        backend.name()
    );
    assert_reference_counts(backend, &record, 0, 0).await;
}

/// A terminal commit with `RetainRollback` moves the live reference into a
/// rollback window in the same transaction.
pub(crate) async fn assert_terminal_commit_retains_rollback_window(backend: &dyn Backend) {
    let executions = backend.execution_store().await;
    let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    let execution_id = ExecutionId::new().to_string();
    let record = install_materialized_pair(backend, 0x71).await;
    let accepted =
        materialize_execution_reference(backend, &scope, &execution_id, &record, 0x71).await;
    assert!(matches!(accepted, StartMaterialization::Accepted { .. }));

    let token = executions
        .acquire_lease(
            &scope,
            &execution_id,
            "terminal-rollback",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire lease")
        .expect("lease must be available for a fresh execution");

    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id.clone())
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"status": "Completed"}))
        .reference_transition(ExecutionReferenceTransition::RetainRollback {
            window_id: [0x71; 16],
            retain_until: chrono::Utc::now() + chrono::TimeDelta::minutes(5),
        })
        .build()
        .expect("terminal rollback batch");
    let outcome = executions
        .commit(batch)
        .await
        .expect("terminal commit must apply");
    assert!(
        matches!(outcome, TransitionOutcome::Applied { .. }),
        "[{}] terminal rollback commit must apply, got {outcome:?}",
        backend.name()
    );
    assert_reference_counts(backend, &record, 0, 1).await;
}

/// A terminal commit that tries to release a reference already moved into a
/// rollback window is rejected before any aggregate state changes.
pub(crate) async fn assert_terminal_commit_rejects_incompatible_reference_transition(
    backend: &dyn Backend,
) {
    let executions = backend.execution_store().await;
    let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    let execution_id = ExecutionId::new().to_string();
    let record = install_materialized_pair(backend, 0x72).await;
    let accepted =
        materialize_execution_reference(backend, &scope, &execution_id, &record, 0x72).await;
    assert!(matches!(accepted, StartMaterialization::Accepted { .. }));

    let token = executions
        .acquire_lease(
            &scope,
            &execution_id,
            "terminal-incompatible",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire lease")
        .expect("lease must be available for a fresh execution");

    let first = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id.clone())
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"status": "Completed"}))
        .reference_transition(ExecutionReferenceTransition::RetainRollback {
            window_id: [0x72; 16],
            retain_until: chrono::Utc::now() + chrono::TimeDelta::minutes(5),
        })
        .build()
        .expect("rollback batch");
    let first_outcome = executions
        .commit(first)
        .await
        .expect("rollback commit must apply");
    assert!(matches!(first_outcome, TransitionOutcome::Applied { .. }));

    // The same lease token is still current; only the reference transition is
    // incompatible now.
    let second = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id.clone())
        .expected_version(1)
        .fencing(token)
        .new_state(serde_json::json!({"status": "Completed", "second": true}))
        .reference_transition(ExecutionReferenceTransition::ReleaseLive)
        .build()
        .expect("incompatible release batch");
    let second_outcome = executions.commit(second).await;
    assert!(
        matches!(second_outcome, Err(StorageError::Internal(_))),
        "[{}] releasing a rollback-window reference must fail, got {second_outcome:?}",
        backend.name()
    );

    let row = executions
        .get(&scope, &execution_id)
        .await
        .expect("read execution after failed transition")
        .expect("execution must still exist");
    assert_eq!(
        row.version,
        1,
        "[{}] a failed reference transition must not advance the execution version",
        backend.name()
    );
    assert_eq!(
        row.state,
        serde_json::json!({"status": "Completed"}),
        "[{}] a failed reference transition must not change the execution state",
        backend.name()
    );
}

/// Expired rollback windows are released by the cleanup primitive and no
/// longer count as references.
pub(crate) async fn assert_expired_rollbacks_are_released(backend: &dyn Backend) {
    let executions = backend.execution_store().await;
    let scope = Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string());
    let execution_id = ExecutionId::new().to_string();
    let record = install_materialized_pair(backend, 0x73).await;
    let accepted =
        materialize_execution_reference(backend, &scope, &execution_id, &record, 0x73).await;
    assert!(matches!(accepted, StartMaterialization::Accepted { .. }));

    let token = executions
        .acquire_lease(
            &scope,
            &execution_id,
            "expired-rollback",
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("acquire lease")
        .expect("lease must be available for a fresh execution");

    let batch = TransitionBatch::builder()
        .scope(scope.clone())
        .execution_id(execution_id.clone())
        .expected_version(0)
        .fencing(token)
        .new_state(serde_json::json!({"status": "Completed"}))
        .reference_transition(ExecutionReferenceTransition::RetainRollback {
            window_id: [0x73; 16],
            retain_until: chrono::Utc::now() - chrono::TimeDelta::minutes(1),
        })
        .build()
        .expect("expired rollback batch");
    let outcome = executions
        .commit(batch)
        .await
        .expect("terminal commit must apply");
    assert!(matches!(outcome, TransitionOutcome::Applied { .. }));

    let admin = backend.plan_flavor_catalog_admin().await;
    let released = admin
        .release_expired_rollbacks(10)
        .await
        .expect("expired rollback release must succeed");
    assert_eq!(
        released,
        1,
        "[{}] exactly one expired rollback window must be released",
        backend.name()
    );
    assert_reference_counts(backend, &record, 0, 0).await;

    let again = admin
        .release_expired_rollbacks(10)
        .await
        .expect("repeat rollback release must succeed");
    assert_eq!(
        again,
        0,
        "[{}] releasing expired rollbacks is idempotent",
        backend.name()
    );
}

// ── job-dispatch conformance assertions ───────────────────────────────────

fn make_job(id: u8, required_plugin_key: &str, tags: &[&str]) -> JobDispatchMsg {
    let key: PluginKey = required_plugin_key
        .parse()
        .expect("conformance test plugin key must be valid");
    let required_plugins: Vec<PluginKey> = tags
        .iter()
        .map(|s| {
            s.parse::<PluginKey>()
                .expect("conformance test tag must be valid")
        })
        .collect();
    JobDispatchMsg::new(
        [id; 16],
        format!("exe_{id}"),
        ControlCommand::Start,
        scope_a(),
        serde_json::json!({}),
        None::<&str>,
        key,
        required_plugins,
        None::<&str>,
        0,
        WorkerFlavorRevisionId::from_bytes([0x11; 32]),
    )
}

/// `claim_pending` only delivers rows whose required plugin is in the worker's
/// `available_plugins`; a row requiring an unavailable plugin is not delivered.
pub(crate) async fn assert_job_dispatch_routes_by_plugin(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;

    let job_a = make_job(0x10, "plugin.alpha", &["plugin.alpha"]);
    let job_b = make_job(0x11, "plugin.beta", &["plugin.beta"]);
    q.enqueue(&job_a).await.expect("enqueue alpha");
    q.enqueue(&job_b).await.expect("enqueue beta");

    let proc = [9u8; 16];
    // Advertise only alpha — must NOT receive beta.
    let claimed = q
        .claim_pending(
            &proc,
            16,
            &["plugin.alpha".parse::<PluginKey>().unwrap()],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim");
    assert_eq!(
        claimed.len(),
        1,
        "[{}] only alpha row claimed",
        backend.name()
    );
    assert_eq!(
        claimed[0].msg.required_plugin_key.as_str(),
        "plugin.alpha",
        "[{}] claimed row must be alpha",
        backend.name()
    );

    // Advertise only beta — beta row is still Pending (alpha took none).
    let claimed_b = q
        .claim_pending(
            &proc,
            16,
            &["plugin.beta".parse::<PluginKey>().unwrap()],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim beta");
    assert_eq!(claimed_b.len(), 1, "[{}] beta row claimed", backend.name());

    // Advertise an unrelated tag — nothing claimed.
    let nothing = q
        .claim_pending(
            &proc,
            16,
            &["plugin.gamma".parse::<PluginKey>().unwrap()],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim gamma");
    assert!(
        nothing.is_empty(),
        "[{}] unadvertised tag must not match any row",
        backend.name()
    );
}

/// The primary plugin remains an independent routing requirement when a
/// malformed decoded message omits it from the full plugin set.
pub(crate) async fn assert_job_dispatch_requires_primary_plugin(backend: &dyn Backend) {
    let queue = backend.job_dispatch_queue().await;
    let mut job = make_job(0x12, "plugin.alpha", &["plugin.alpha"]);
    job.required_plugins = vec![
        "plugin.beta"
            .parse::<PluginKey>()
            .expect("conformance test plugin key must be valid"),
    ];
    queue.enqueue(&job).await.expect("enqueue malformed job");

    let claims = queue
        .claim_pending(
            &[9; 16],
            16,
            &["plugin.beta"
                .parse::<PluginKey>()
                .expect("conformance test plugin key must be valid")],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim malformed job");

    assert!(
        claims.is_empty(),
        "[{}] a worker missing the primary plugin must not claim malformed routing metadata",
        backend.name()
    );
}

/// Terminal retention starts at the terminal transition, not when the claim
/// began. SQL fixtures move timestamps directly so the assertion has no
/// wall-clock sleep or scheduler race.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) async fn assert_sql_job_cleanup_uses_terminal_transition(
    backend: &dyn SqlJobTimestampFixture,
) {
    let queue = backend.job_dispatch_queue().await;
    let job = make_job(0x13, "plugin.alpha", &["plugin.alpha"]);
    queue.enqueue(&job).await.expect("enqueue cleanup job");
    let claim = queue
        .claim_pending(
            &[9; 16],
            1,
            &["plugin.alpha"
                .parse::<PluginKey>()
                .expect("conformance test plugin key must be valid")],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim cleanup job")
        .remove(0);

    backend
        .age_job_timestamp(&job.id, std::time::Duration::from_secs(10))
        .await
        .expect("age active claim timestamp");
    queue
        .mark_dispatched(&claim.token)
        .await
        .expect("terminalize cleanup job");
    assert_eq!(
        queue
            .cleanup(std::time::Duration::from_secs(1))
            .await
            .expect("cleanup immediately after terminal transition"),
        0,
        "[{}] a long-running claim must survive immediately after terminalization",
        backend.name()
    );

    backend
        .age_job_timestamp(&job.id, std::time::Duration::from_secs(10))
        .await
        .expect("age terminal timestamp");
    assert_eq!(
        queue
            .cleanup(std::time::Duration::from_secs(1))
            .await
            .expect("cleanup expired terminal job"),
        1,
        "[{}] an expired terminal job must be deleted",
        backend.name()
    );
}

/// `mark_dispatched` and `mark_failed` are both fenced on the storage-minted
/// claim generation: a token that does not name the row's current claim
/// changes nothing.
pub(crate) async fn assert_job_dispatch_fencing(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;
    let plugin_tags = &["plugin.x".parse::<PluginKey>().unwrap()];

    let runner_a = [1u8; 16];

    // ── mark_dispatched fencing ───────────────────────────────────────────────
    let job_d = make_job(0x20, "plugin.x", &["plugin.x"]);
    q.enqueue(&job_d).await.expect("enqueue job_d");

    let claimed = q
        .claim_pending(
            &runner_a,
            16,
            plugin_tags,
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim");
    assert_eq!(claimed.len(), 1, "[{}] claimed one row", backend.name());
    let current = &claimed[0].token;

    let foreign_scope = JobClaimToken::new(job_d.id, current.generation(), scope_b());
    let foreign_dispatch = q.mark_dispatched(&foreign_scope).await;
    assert!(
        matches!(
            foreign_dispatch,
            Err(StorageError::NotFound {
                entity: "job_dispatch",
                ..
            })
        ),
        "[{}] a job claim bound to another tenant must receive uniform NotFound, got: {foreign_dispatch:?}",
        backend.name()
    );

    // A token naming a generation this row never reached must be rejected.
    // Under the old `processed_by` fence this was expressed as "a different
    // processor id"; authority is now the token, which a caller cannot forge
    // into existence, so the equivalent probe is a wrong generation.
    let forged = JobClaimToken::new(
        job_d.id,
        ClaimGeneration::new(current.generation().get() + 1),
        current.scope().clone(),
    );
    let stale_dispatched = q.mark_dispatched(&forged).await;
    assert!(
        matches!(stale_dispatched, Err(StorageError::FencedOut { .. })),
        "[{}] mark_dispatched with a non-current generation must be FencedOut, got: {:?}",
        backend.name(),
        stale_dispatched
    );

    // The row is still Processing (the fenced call made no change) — the
    // holder of the current claim succeeds.
    q.mark_dispatched(current)
        .await
        .expect("mark_dispatched (claimant)");

    // After mark_dispatched, a fresh claim should find no pending rows.
    let after_dispatch = q
        .claim_pending(
            &runner_a,
            16,
            plugin_tags,
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim after dispatch");
    assert!(
        after_dispatch.is_empty(),
        "[{}] no pending rows after mark_dispatched",
        backend.name()
    );

    // ── mark_failed fencing ───────────────────────────────────────────────────
    let job_f = make_job(0x21, "plugin.x", &["plugin.x"]);
    q.enqueue(&job_f).await.expect("enqueue job_f");

    let claimed_f = q
        .claim_pending(
            &runner_a,
            16,
            plugin_tags,
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim job_f");
    assert_eq!(claimed_f.len(), 1, "[{}] claimed job_f", backend.name());
    let current_f = &claimed_f[0].token;

    let forged_f = JobClaimToken::new(
        job_f.id,
        ClaimGeneration::new(current_f.generation().get() + 1),
        current_f.scope().clone(),
    );
    let stale_failed = q.mark_failed(&forged_f, "stale error").await;
    assert!(
        matches!(stale_failed, Err(StorageError::FencedOut { .. })),
        "[{}] mark_failed with a non-current generation must be FencedOut, got: {:?}",
        backend.name(),
        stale_failed
    );

    // The row is still Processing — the current claim can still fail it.
    q.mark_failed(current_f, "real error")
        .await
        .expect("mark_failed (claimant)");

    // After mark_failed the row is terminal; fresh claim yields nothing.
    let after_failed = q
        .claim_pending(
            &runner_a,
            16,
            plugin_tags,
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim after failed");
    assert!(
        after_failed.is_empty(),
        "[{}] no pending rows after mark_failed",
        backend.name()
    );
}

/// How long a claim must age before `reclaim_stuck` will take it back.
///
/// The SQL backends compare wall-clock epoch-millis while the in-memory
/// backend compares `tokio::time::Instant`s; a real sleep advances both, so
/// one shared assertion can drive all three. The margin is generous because
/// the assertion is about ordering, not about a deadline.
const ABA_RECLAIM_HORIZON: std::time::Duration = std::time::Duration::from_millis(20);
const ABA_CLAIM_AGE: std::time::Duration = std::time::Duration::from_millis(60);

/// A stable processor identity cannot acknowledge a claim that a reclaim
/// already superseded by a newer claim from the same processor.
///
/// One processor claims a row, the sweep hands the row back, and the *same*
/// processor claims it again. Every `processed_by`-based fence accepts an
/// acknowledgement issued against the first claim at that point, because the
/// recorded processor still matches: the late acknowledgement terminalises a
/// row the second attempt is still working. Only a per-attempt generation
/// tells the two apart.
pub(crate) async fn assert_job_dispatch_same_processor_aba_is_fenced(
    backend: &dyn Backend,
) -> serde_json::Value {
    let q = backend.job_dispatch_queue().await;
    let plugin_tags = &["plugin.aba".parse::<PluginKey>().unwrap()];
    // One identity for both attempts — that is the whole point.
    let processor = [7u8; 16];

    let job = make_job(0x22, "plugin.aba", &["plugin.aba"]);
    q.enqueue(&job).await.expect("enqueue aba job");

    let first = q
        .claim_pending(
            &processor,
            16,
            plugin_tags,
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("first claim");
    assert_eq!(
        first.len(),
        1,
        "[{}] generation N must claim the row",
        backend.name()
    );
    let superseded = first[0].token.clone();

    tokio::time::sleep(ABA_CLAIM_AGE).await;
    let outcome = q
        .reclaim_stuck(ABA_RECLAIM_HORIZON, 16)
        .await
        .expect("reclaim");
    assert_eq!(
        (outcome.reclaimed, outcome.exhausted),
        (1, 0),
        "[{}] the aged claim must be reclaimed, not exhausted",
        backend.name()
    );

    let second = q
        .claim_pending(
            &processor,
            16,
            plugin_tags,
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("second claim");
    assert_eq!(
        second.len(),
        1,
        "[{}] generation N+1 must re-claim the row",
        backend.name()
    );
    let current = second[0].token.clone();
    assert_eq!(
        current.row_id(),
        superseded.row_id(),
        "[{}] both attempts must name the same row",
        backend.name()
    );
    assert!(
        current.generation() > superseded.generation(),
        "[{}] a re-claim must mint a strictly greater generation ({} then {})",
        backend.name(),
        superseded.generation(),
        current.generation()
    );

    let late_ack = q.mark_dispatched(&superseded).await;
    assert!(
        matches!(late_ack, Err(StorageError::FencedOut { .. })),
        "[{}] a late ack from generation N must be FencedOut, got: {late_ack:?}",
        backend.name()
    );
    let late_nack = q
        .mark_failed(&superseded, "late generation-N failure")
        .await;
    assert!(
        matches!(late_nack, Err(StorageError::FencedOut { .. })),
        "[{}] a late nack from generation N must be FencedOut, got: {late_nack:?}",
        backend.name()
    );

    // Zero state change: the row is still owned by generation N+1, which can
    // still acknowledge it. Were the fence merely returning an error while
    // terminalising the row, this would fail.
    q.mark_dispatched(&current)
        .await
        .expect("generation N+1 still owns the row after the fenced acknowledgements");
    serde_json::json!({
        "queue": "job",
        "row_id": current.row_id(),
        "superseded_generation": superseded.generation().get(),
        "current_generation": current.generation().get(),
        "same_processor": true,
        "late_ack_fenced": true,
        "late_nack_fenced": true,
        "stale_generation_mutation_count": 0,
        "current_owner_completed_after_stale_attempts": true
    })
}

/// `claim_pending` enforces a superset predicate: a worker may claim a job
/// only when its `available_plugins` cover EVERY plugin in the job's `required_plugins`.
///
/// Contract (job has `required_plugin_key = "plugin.alpha"` and
/// `required_plugins = ["plugin.alpha", "plugin.beta"]`):
///
/// 1. Advertised `["plugin.alpha"]` only → NOT claimed (missing beta).
/// 2. Advertised `["plugin.beta"]` only → NOT claimed (missing alpha; the
///    `required_plugin_key` pre-filter also rejects it independently).
/// 3. Advertised `["plugin.alpha", "plugin.beta"]` → claimed (exact superset).
/// 4. Advertised `["plugin.alpha", "plugin.beta", "plugin.gamma"]` → claimed
///    (strict superset); claimed job identity verified.
/// 5. Empty advertised set → claims nothing (parity across all backends).
pub(crate) async fn assert_job_dispatch_routes_by_plugin_superset(backend: &dyn Backend) {
    let q = backend.job_dispatch_queue().await;

    // Job requires alpha AND beta (required_plugins covers both; invariant upheld).
    let job = make_job(0x60, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job).await.expect("enqueue superset job");

    let proc = [0xAAu8; 16];

    // 1. Alpha-only worker: pre-filter passes (required_plugin_key = alpha) but
    //    superset check fails — beta is required and not advertised.
    let claimed_by_alpha_only = q
        .claim_pending(
            &proc,
            16,
            &["plugin.alpha".parse::<PluginKey>().unwrap()],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim alpha-only");
    assert!(
        claimed_by_alpha_only.is_empty(),
        "[{}] alpha-only worker must not claim a job that also requires beta \
         (superset predicate: all required_plugins must be covered)",
        backend.name()
    );

    // 2. Beta-only worker: pre-filter rejects (required_plugin_key = alpha not
    //    in advertised) and the superset check would also fail independently.
    let claimed_by_beta_only = q
        .claim_pending(
            &proc,
            16,
            &["plugin.beta".parse::<PluginKey>().unwrap()],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim beta-only");
    assert!(
        claimed_by_beta_only.is_empty(),
        "[{}] beta-only worker must not claim an alpha-required job \
         (pre-filter on required_plugin_key rejects it)",
        backend.name()
    );

    // 3. Exact-superset worker: advertises both alpha and beta — must claim.
    let claimed_by_both = q
        .claim_pending(
            &proc,
            16,
            &[
                "plugin.alpha".parse::<PluginKey>().unwrap(),
                "plugin.beta".parse::<PluginKey>().unwrap(),
            ],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim alpha+beta");
    assert_eq!(
        claimed_by_both.len(),
        1,
        "[{}] worker advertising [alpha, beta] must claim the job (exact superset)",
        backend.name()
    );
    assert_eq!(
        claimed_by_both[0].msg.required_plugin_key.as_str(),
        "plugin.alpha",
        "[{}] claimed job must be the alpha-required one",
        backend.name()
    );

    // 4. Strict-superset worker (re-enqueue to get a fresh Pending row).
    let job2 = make_job(0x61, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job2).await.expect("enqueue superset job 2");
    let claimed_by_superset = q
        .claim_pending(
            &proc,
            16,
            &[
                "plugin.alpha".parse::<PluginKey>().unwrap(),
                "plugin.beta".parse::<PluginKey>().unwrap(),
                "plugin.gamma".parse::<PluginKey>().unwrap(),
            ],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim strict superset");
    assert_eq!(
        claimed_by_superset.len(),
        1,
        "[{}] worker advertising [alpha, beta, gamma] must claim a job requiring [alpha, beta]",
        backend.name()
    );
    assert_eq!(
        claimed_by_superset[0].msg.id,
        job2.id,
        "[{}] claimed job must be the strict-superset job (id match)",
        backend.name()
    );
    assert_eq!(
        claimed_by_superset[0].msg.required_plugin_key.as_str(),
        "plugin.alpha",
        "[{}] claimed job must be the alpha-required one",
        backend.name()
    );

    // 5. Empty advertised set → claims nothing — parity with SQLite + Postgres
    //    which both short-circuit on empty available_plugins.  Re-enqueue a
    //    conforming job (required_plugins ⊇ {required_plugin_key}) to confirm
    //    it stays Pending.
    let job3 = make_job(0x62, "plugin.alpha", &["plugin.alpha", "plugin.beta"]);
    q.enqueue(&job3)
        .await
        .expect("enqueue job for empty-advertised check");
    let claimed_empty_adv = q
        .claim_pending(
            &proc,
            16,
            &[],
            WorkerFlavorRevisionId::from_bytes([0x11; 32]),
        )
        .await
        .expect("claim with empty advertised");
    assert!(
        claimed_empty_adv.is_empty(),
        "[{}] empty advertised set must claim nothing (parity with SQL backends)",
        backend.name()
    );
}

/// A released claim is immediately re-claimable, is fenced against a
/// superseded token, and does not consume the reclaim budget.
///
/// `release_claim` exists so a dispatch that hit momentary contention retries
/// on the next poll rather than waiting out `reclaim_after`, which is sized in
/// minutes to detect a runner that *died*. Every backend has to agree on all
/// three properties, because a `Cancel` that lost a millisecond-wide race is
/// the case this serves, and a per-backend difference would be a latency cliff
/// no operator could attribute.
pub(crate) async fn assert_control_queue_release_returns_row_for_redelivery(backend: &dyn Backend) {
    let store = backend.execution_store().await;
    let queue = backend.control_queue().await;
    let scope = scope_a();
    let processor = [21u8; 16];

    let execution_id = "exe_control_release";
    store
        .create(
            &scope,
            execution_id,
            "wf_control_release",
            serde_json::json!({}),
        )
        .await
        .expect("create the execution the command targets");
    queue
        .enqueue(&ControlMsg {
            id: [0x5Bu8; 16],
            execution_id: execution_id.to_owned(),
            command: ControlCommand::Cancel,
            scope: scope.clone(),
            w3c_traceparent: None,
            reclaim_count: 0,
            resume_target: None,
        })
        .await
        .expect("enqueue the control command");

    let first = queue
        .claim_pending(&processor, 16)
        .await
        .expect("first claim");
    assert_eq!(
        first.len(),
        1,
        "[{}] the command must claim",
        backend.name()
    );
    let released = first[0].token.clone();

    queue
        .release_claim(&released)
        .await
        .expect("releasing an owned claim must succeed");

    // Re-claimable at once — no reclaim sweep, no waiting.
    let second = queue
        .claim_pending(&processor, 16)
        .await
        .expect("second claim");
    assert_eq!(
        second.len(),
        1,
        "[{}] a released claim must be re-claimable immediately, without a reclaim sweep",
        backend.name()
    );
    assert_eq!(
        second[0].msg.reclaim_count,
        0,
        "[{}] releasing must not spend the reclaim budget — a retry is not a stuck row",
        backend.name()
    );
    assert!(
        second[0].token.generation() > released.generation(),
        "[{}] the re-claim must mint a fresh generation so the released token is dead",
        backend.name()
    );

    // The superseded token cannot release the row out from under its new owner.
    let stale = queue.release_claim(&released).await;
    assert!(
        matches!(stale, Err(StorageError::FencedOut { .. })),
        "[{}] a superseded claim must not release a row another processor now owns; got {stale:?}",
        backend.name()
    );

    // And the row is still owned by the second claim, not sitting Pending.
    let third = queue
        .claim_pending(&processor, 16)
        .await
        .expect("third claim");
    assert!(
        third.is_empty(),
        "[{}] the fenced-out release must have changed nothing",
        backend.name()
    );
}

/// Exact flavor is persisted by both insert routes, filters before the batch
/// limit, and survives reclaim without giving an old claim ABA authority.
pub(crate) async fn assert_job_dispatch_exact_flavor(backend: &dyn Backend) {
    use std::time::Duration;
    let queue = backend.job_dispatch_queue().await;
    let matching_flavor = WorkerFlavorRevisionId::from_bytes([0x61; 32]);
    let other_flavor = WorkerFlavorRevisionId::from_bytes([0x62; 32]);
    let plugins = ["exact.flavor".parse::<PluginKey>().unwrap()];
    let processor = [0x63; 16];
    let mut wrong = make_job(0x61, "exact.flavor", &["exact.flavor"]);
    wrong.required_worker_flavor_id = other_flavor;
    let mut matching = make_job(0x62, "exact.flavor", &["exact.flavor"]);
    matching.required_worker_flavor_id = matching_flavor;
    queue.enqueue(&wrong).await.unwrap();
    queue.enqueue(&matching).await.unwrap();

    let claims = queue
        .claim_pending(&processor, 1, &plugins, matching_flavor)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(
        claims[0].msg, matching,
        "compose must persist exact identity and filter before LIMIT"
    );
    let stale = claims[0].token.clone();
    assert!(
        queue
            .claim_pending(&processor, 1, &plugins, matching_flavor)
            .await
            .unwrap()
            .is_empty()
    );

    // Both wall-clock SQL backends and the monotonic reference backend cross
    // the strict reclaim cutoff. Only this uniquely scoped queue is reclaimed.
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(
        queue
            .reclaim_stuck(Duration::ZERO, 3)
            .await
            .unwrap()
            .reclaimed,
        1
    );
    let reclaimed = queue
        .claim_pending(&processor, 1, &plugins, matching_flavor)
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].msg.required_worker_flavor_id, matching_flavor);
    assert_eq!(reclaimed[0].msg.reclaim_count, 1);
    assert!(reclaimed[0].token.generation() > stale.generation());
    assert!(matches!(
        queue.mark_dispatched(&stale).await,
        Err(StorageError::FencedOut { .. })
    ));
    queue.mark_dispatched(&reclaimed[0].token).await.unwrap();

    let other = queue
        .claim_pending(&processor, 1, &plugins, other_flavor)
        .await
        .unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(
        other[0].msg, wrong,
        "ordinary enqueue must preserve exact identity"
    );
    queue.mark_dispatched(&other[0].token).await.unwrap();
}
