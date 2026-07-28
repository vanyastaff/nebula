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
mod execution;
mod idempotency_store;
mod identity;
mod job_dispatch;
mod resume_producer;
mod resume_token;
mod workflow;

pub use control_queue::{SqliteControlQueue, SqliteJournalReader};
pub use execution::{SqliteExecutionStore, SqliteIdempotencyGuard};
pub use idempotency_store::{SqliteIdempotencyStore, SqliteWebhookActivationStore};
pub use identity::{
    SqliteAuditStore, SqliteBlobStore, SqliteMembershipStore, SqliteOrgStore, SqliteQuotaStore,
    SqliteResourceStore, SqliteTriggerStore, SqliteUserStore, SqliteWorkspaceStore,
};
pub use job_dispatch::{SqliteJobDispatchQueue, SqliteTriggerDedupInbox};
pub use resume_producer::SqliteResumeProducer;
pub use resume_token::SqliteResumeTokenStore;
pub use workflow::{SqliteWorkflowStore, SqliteWorkflowVersionStore};

/// Admit a canonical schema and apply every pending ordered migration under
/// the serialized Nebula SQLite setup guard.
///
/// # Errors
/// Returns a closed, redacted connection or configuration error if setup
/// cannot prove that the database has a supported canonical migration history
/// at the catalog-only upgrade floor and enabled foreign-key enforcement.
pub async fn init_schema(pool: &sqlx::SqlitePool) -> Result<(), nebula_storage_port::StorageError> {
    crate::migration::setup_sqlite_pool(pool.clone())
        .await
        .map_err(crate::migration::storage_setup_error)
}
