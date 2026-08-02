//! Postgres adapter — the `nebula-storage-port` implementation for
//! production (multi-process, restart-tolerant).
//!
//! Per spec §5: `commit` uses a real transaction so the §12.2 triple
//! (CAS + fencing check + state + outbox + journal) is atomic and
//! serializable across processes; the control-queue claim uses
//! `FOR UPDATE SKIP LOCKED` (multi-consumer queue claim) — wired by the
//! control-queue store in a later task.
//!
//! The adapter schema is installed exclusively by the ordered PostgreSQL
//! migration catalog. The `port_*` execution core remains independent of
//! identity seeding.

mod control_queue;
mod execution;
mod idempotency_store;
mod identity;
mod job_dispatch;
mod operation_ledger;
mod plan_flavor_catalog;
mod resume_producer;
mod resume_token;
mod start_acceptance;
mod workflow;

pub use control_queue::{PgControlQueue, PgJournalReader};
pub use execution::{PgExecutionStore, PgIdempotencyGuard};
pub use idempotency_store::{PgIdempotencyStore, PgWebhookActivationStore};
pub use identity::{
    PgAuditStore, PgBlobStore, PgMembershipStore, PgOrgStore, PgQuotaStore, PgResourceStore,
    PgTriggerStore, PgUserStore, PgWorkspaceStore,
};
pub use job_dispatch::{PgJobDispatchQueue, PgTriggerDedupInbox};
pub use operation_ledger::PgOperationLedger;
pub use plan_flavor_catalog::PgPlanFlavorCatalog;
pub use resume_producer::PgResumeProducer;
pub use resume_token::PgResumeTokenStore;
pub use start_acceptance::PgStartAcceptanceStore;
pub use workflow::{PgWorkflowStore, PgWorkflowVersionStore};

/// Admit a canonical schema and apply every pending ordered migration under
/// the PostgreSQL setup lock with bounded acquisition and release.
///
/// # Errors
/// Returns a closed, redacted connection or configuration error if setup
/// cannot prove that the database has a canonical migration history at the
/// catalog-only upgrade floor.
/// Migration duration follows the caller lifecycle and the operator's database
/// `statement_timeout`.
pub async fn init_schema(pool: &sqlx::PgPool) -> Result<(), nebula_storage_port::StorageError> {
    crate::migration::setup_postgres_pool(pool.clone())
        .await
        .map_err(crate::migration::storage_setup_error)
}

/// Adopt a database provisioned before the ordered migration ledger existed.
///
/// See [`crate::sqlite::adopt_ledger`] for the full contract; this is the
/// PostgreSQL entry point and behaves identically.
///
/// # Errors
/// Returns [`crate::LedgerAdoptionError`] if the database cannot be read or written,
/// if `through_version` names no canonical migration, or if the stamped ledger
/// would still be rejected by schema setup.
pub async fn adopt_ledger(
    pool: &sqlx::PgPool,
    through_version: i64,
) -> Result<crate::LedgerAdoptionOutcome, crate::LedgerAdoptionError> {
    crate::migration::adopt_postgres_ledger(pool, through_version).await
}
