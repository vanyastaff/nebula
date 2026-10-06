//! SQLite adapter — the `nebula-storage-port` implementation for dev / edge
//! single-writer deployments.
//!
//! Per spec §5: SQLite parity is **identical port API + single-writer
//! correctness**, explicitly NOT concurrent/throughput parity. `commit`
//! opens a `BEGIN IMMEDIATE` transaction so the CAS + fencing + state +
//! outbox + journal triple is atomic against the single writer; the
//! control-queue claim is a single-consumer status flip (no
//! `FOR UPDATE SKIP LOCKED` equivalent — documented, not hidden).
//!
//! The adapter schema is installed exclusively by the ordered SQLite
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
mod resource_runtime;
mod resource_status;
mod resume_producer;
mod resume_token;
mod start_acceptance;
mod turn_handoff;
mod turn_recovery;
mod webhook_activation;
mod workflow;

pub use control_queue::{SqliteControlQueue, SqliteJournalReader};
pub use execution::{SqliteExecutionStore, SqliteIdempotencyGuard};
pub use identity::{
    SqliteMembershipStore, SqliteOrgStore, SqliteResourceStore, SqliteTenantProvisioningStore,
    SqliteTriggerStore, SqliteWorkspaceStore,
};
pub use iteration_checkpoint::SqliteCheckpointStore;
pub use job_dispatch::SqliteJobDispatchQueue;
pub use operation_ledger::SqliteOperationLedger;
pub use plan_flavor_catalog::SqlitePlanFlavorCatalog;
pub use resource_runtime::SqliteResourceRuntime;
pub use resource_status::SqliteResourceStatusStore;
pub use resume_producer::SqliteResumeProducer;
pub use resume_token::SqliteResumeTokenStore;
pub use start_acceptance::SqliteStartAcceptanceStore;
pub use turn_handoff::SqliteTurnHandoff;
pub use webhook_activation::SqliteWebhookActivationStore;
pub use workflow::{SqliteWorkflowStore, SqliteWorkflowVersionStore};

/// Admit a canonical schema and apply every pending ordered migration under
/// the serialized Nebula SQLite setup guard.
///
/// # Errors
/// Returns a closed, redacted connection error if the database or its setup
/// lock is unavailable, and a configuration error naming the rejection if the
/// migration ledger is not a canonical prefix of this build's catalog (a
/// database created by another build or edited by hand; reset it).
pub async fn init_schema(pool: &sqlx::SqlitePool) -> Result<(), nebula_storage_port::StorageError> {
    crate::migration::setup_sqlite_pool(pool.clone())
        .await
        .map_err(crate::migration::storage_setup_error)
}
