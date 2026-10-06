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
mod control_turn;
mod execution;
mod execution_fence;
mod identity;
mod iteration_checkpoint;
mod job_dispatch;
mod operation_ledger;
mod plan_flavor_catalog;
mod rate_limit;
mod resource_runtime;
mod resource_status;
mod resume_producer;
mod resume_token;
mod start_acceptance;
mod turn_handoff;
mod turn_recovery;
mod webhook_activation;
mod workflow;

pub use control_queue::{PgControlQueue, PgJournalReader};
pub use execution::{PgExecutionStore, PgIdempotencyGuard};
pub use identity::{
    PgMembershipStore, PgOrgStore, PgResourceStore, PgTenantProvisioningStore, PgTriggerStore,
    PgWorkspaceStore,
};
pub use iteration_checkpoint::PgCheckpointStore;
pub use job_dispatch::PgJobDispatchQueue;
pub use operation_ledger::PgOperationLedger;
pub use plan_flavor_catalog::PgPlanFlavorCatalog;
pub use rate_limit::PgLimitStore;
pub use resource_runtime::PgResourceRuntime;
pub use resource_status::PgResourceStatusStore;
pub use resume_producer::PgResumeProducer;
pub use resume_token::PgResumeTokenStore;
pub use start_acceptance::PgStartAcceptanceStore;
pub use turn_handoff::PgTurnHandoff;
pub use webhook_activation::PgWebhookActivationStore;
pub use workflow::{PgWorkflowStore, PgWorkflowVersionStore};

/// Admit a canonical schema and apply every pending ordered migration under
/// the PostgreSQL setup lock with bounded acquisition and release.
///
/// # Errors
/// Returns a closed, redacted connection error if the database or its setup
/// lock is unavailable, and a configuration error naming the rejection if the
/// migration ledger is not a canonical prefix of this build's catalog (a
/// database created by another build or edited by hand; reset it).
/// Migration duration follows the caller lifecycle and the operator's database
/// `statement_timeout`.
pub async fn init_schema(pool: &sqlx::PgPool) -> Result<(), nebula_storage_port::StorageError> {
    crate::migration::setup_postgres_pool(pool.clone())
        .await
        .map_err(crate::migration::storage_setup_error)
}
