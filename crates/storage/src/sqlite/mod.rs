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
mod plan_flavor_catalog;
mod resume_producer;
mod resume_token;
mod start_acceptance;
mod workflow;

pub use control_queue::{SqliteControlQueue, SqliteJournalReader};
pub use execution::{SqliteExecutionStore, SqliteIdempotencyGuard};
pub use idempotency_store::{SqliteIdempotencyStore, SqliteWebhookActivationStore};
pub use identity::{
    SqliteAuditStore, SqliteBlobStore, SqliteMembershipStore, SqliteOrgStore, SqliteQuotaStore,
    SqliteResourceStore, SqliteTriggerStore, SqliteUserStore, SqliteWorkspaceStore,
};
pub use job_dispatch::{SqliteJobDispatchQueue, SqliteTriggerDedupInbox};
pub use plan_flavor_catalog::SqlitePlanFlavorCatalog;
pub use resume_producer::SqliteResumeProducer;
pub use resume_token::SqliteResumeTokenStore;
pub use start_acceptance::SqliteStartAcceptanceStore;
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

/// Adopt a database provisioned before the ordered migration ledger existed.
///
/// Databases created by the previous idempotent `init_schema` carry the
/// `port_*` schema with no `_sqlx_migrations` ledger, so [`init_schema`] now
/// refuses them and the owning process cannot start. This stamps a ledger
/// recording migrations `1..=through_version` as already applied, which is an
/// operator assertion that the live schema is what those migrations produce —
/// it is deliberately never performed automatically at startup.
///
/// The stamp runs in one transaction and the resulting ledger is re-admitted
/// before commit, so a database that would still be rejected is left exactly
/// as it was rather than carrying a half-written ledger.
///
/// # Errors
/// Returns [`crate::LedgerAdoptionError`] if the database cannot be read or written,
/// if `through_version` names no canonical migration, or if the stamped ledger
/// would still be rejected by schema setup.
pub async fn adopt_ledger(
    pool: &sqlx::SqlitePool,
    through_version: i64,
) -> Result<crate::LedgerAdoptionOutcome, crate::LedgerAdoptionError> {
    crate::migration::adopt_sqlite_ledger(pool, through_version).await
}
