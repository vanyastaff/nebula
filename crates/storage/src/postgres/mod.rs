//! Postgres adapter — the `nebula-storage-port` implementation for
//! production (multi-process, restart-tolerant).
//!
//! Per spec §5: `commit` uses a real transaction so the §12.2 triple
//! (CAS + fencing check + state + outbox + journal) is atomic and
//! serializable across processes; the control-queue claim uses
//! `FOR UPDATE SKIP LOCKED` (multi-consumer queue claim) — wired by the
//! control-queue store in a later task.
//!
//! The adapter owns a port-scoped schema (`schema.sql`, `port_*` tables)
//! that does not FK into the identity zoo, so the execution core works on
//! a bare database with no identity seeding.

mod control_queue;
mod execution;
mod idempotency_store;
mod identity;
mod workflow;

pub use control_queue::{PgControlQueue, PgJournalReader};
pub use execution::{PgExecutionStore, PgIdempotencyGuard};
pub use idempotency_store::{PgIdempotencyStore, PgWebhookActivationStore};
pub use identity::{
    PgAuditStore, PgBlobStore, PgMembershipStore, PgOrgStore, PgQuotaStore, PgResourceStore,
    PgTriggerStore, PgUserStore, PgWorkspaceStore,
};
pub use workflow::{PgWorkflowStore, PgWorkflowVersionStore};

/// Embedded port-scoped DDL applied by [`init_schema`].
pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Apply the port-scoped schema to a pool. Idempotent (`CREATE TABLE IF
/// NOT EXISTS`), safe to call on every adapter construction.
///
/// # Errors
/// Returns [`nebula_storage_port::StorageError::Connection`] if the DDL
/// cannot be applied.
pub async fn init_schema(pool: &sqlx::PgPool) -> Result<(), nebula_storage_port::StorageError> {
    // Strip `--` line comments before splitting on `;` (a raw `;` split
    // would otherwise yield fragments that begin mid-comment).
    let stripped: String = SCHEMA_SQL
        .lines()
        .map(|line| match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");
    for stmt in stripped.split(';') {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        sqlx::query(sqlx::AssertSqlSafe(trimmed))
            .execute(pool)
            .await
            .map_err(|e| nebula_storage_port::StorageError::Connection(e.to_string()))?;
    }
    Ok(())
}
