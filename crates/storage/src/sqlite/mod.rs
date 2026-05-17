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
//! The adapter owns a port-scoped schema (`schema.sql`) that does not FK
//! into the identity zoo, so the execution core works on a bare `:memory:`
//! database with no identity seeding.

mod control_queue;
mod execution;
mod idempotency_store;
mod workflow;

pub use control_queue::{SqliteControlQueue, SqliteJournalReader};
pub use execution::{SqliteExecutionStore, SqliteIdempotencyGuard};
pub use idempotency_store::{SqliteIdempotencyStore, SqliteWebhookActivationStore};
pub use workflow::{SqliteWorkflowStore, SqliteWorkflowVersionStore};

/// Embedded port-scoped DDL applied by [`init_schema`].
pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Apply the port-scoped schema to a pool. Idempotent (`CREATE TABLE IF
/// NOT EXISTS`), so it is safe to call on every adapter construction.
///
/// # Errors
/// Returns [`nebula_storage_port::StorageError::Connection`] if the DDL
/// cannot be applied (e.g. a closed pool).
pub async fn init_schema(pool: &sqlx::SqlitePool) -> Result<(), nebula_storage_port::StorageError> {
    // Strip `--` line comments first: splitting raw text on `;` would
    // otherwise yield fragments that begin mid-comment and fail to parse.
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
        sqlx::query(trimmed)
            .execute(pool)
            .await
            .map_err(|e| nebula_storage_port::StorageError::Connection(e.to_string()))?;
    }
    Ok(())
}
