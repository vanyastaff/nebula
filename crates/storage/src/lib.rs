//! # nebula-storage — Storage adapters
//!
//! Concrete persistence **adapters** for the Nebula execution engine. The
//! production persistence *contract* is the spec-16 port
//! (`nebula-storage-port`); this crate provides the in-memory, SQLite, and
//! PostgreSQL implementations of it.
//!
//! ## Port adapters (use these today)
//!
//! - [`inmem`] — in-memory adapter (tests / local / single-process).
//!   Top-level re-exports: `InMemoryExecutionStore`,
//!   `InMemoryWorkflowStore`, `InMemoryWorkflowVersionStore`,
//!   `InMemoryCheckpointStore`, `InMemoryJournalReader`,
//!   `InMemoryNodeResultStore`, `InMemoryIdempotencyGuard`,
//!   `InMemoryIdempotencyStore`, `InMemoryControlQueue`,
//!   `InMemoryWebhookActivationStore`.
//! - `sqlite` — SQLite adapter behind the `sqlite` feature
//!   (dev / edge single-writer; spec §5 SQLite parity boundary).
//! - `postgres` — PostgreSQL adapter behind the `postgres` feature
//!   (production multi-process; real tx + `FOR UPDATE SKIP LOCKED`).
//!
//! This is the layer the knife scenario (`docs/PRODUCT_CANON.md` §13)
//! exercises end-to-end.
//!
//! ## `repos` module — residual non-port surface
//!
//! The persistence concerns that have **not** moved onto the port
//! contract: the durable control-command outbox
//! (`repos::ControlQueueRepo` + `repos::InMemoryControlQueueRepo`,
//! `pg::PgControlQueueRepo` behind the `postgres` feature), the
//! idempotency-cache store (`repos::IdempotencyStoreRepo`, consumed by
//! the API idempotency middleware), the webhook-activation store, and the
//! identity-row repository surface implemented by the Postgres glue in
//! `pg`. `pg::PgControlQueueRepo` is the multi-process /
//! restart-tolerant control-queue backing (`FOR UPDATE SKIP LOCKED`); no
//! composition root selects it by default yet — the server binary wires it in.
//!
//! ## Durability surfaces (port)
//!
//! - CAS transitions via the port `ExecutionStore` commit path.
//! - Per-attempt idempotency via the port `IdempotencyGuard`.
//! - Journal (`append_journal`) and checkpoint (`save_stateful_checkpoint`).
//! - Outbox atomicity: `execution_control_queue` writes share the same operation as state
//!   transitions.
//! - Local path: SQLite is the default; `test_support` provides `sqlite_memory_*` helpers for
//!   in-process tests.
//! - Resume persistence: `set_workflow_input` / `get_workflow_input` and
//!   `save_node_result` / `load_node_result` / `load_all_results` expose the seam for engine
//!   resume wiring.
//!
//! See `crates/storage/README.md` for the full durability matrix and
//! backend status table.

#![warn(missing_docs)]
#![warn(clippy::all)]

/// Credential persistence (encryption, audit, refresh claims, pending state).
pub mod credential;
mod error;
/// Serialization format abstraction (JSON / MessagePack).
pub mod format;
/// In-memory adapter implementing the `nebula-storage-port` contract.
pub mod inmem;
/// Row-to-domain type conversion utilities.
pub mod mapping;
/// Postgres implementations of [`repos`] traits.
#[cfg(feature = "postgres")]
pub mod pg;
/// Connection pool configuration.
pub mod pool;
/// Postgres adapter implementing the `nebula-storage-port` contract
/// (production multi-process; real tx + `FOR UPDATE SKIP LOCKED`).
#[cfg(feature = "postgres")]
pub mod postgres;
/// Backend repository traits for the persistence concerns that have not
/// yet moved onto the `nebula-storage-port` contract.
///
/// Execution and workflow persistence are served by the spec-16 port
/// adapters (`inmem` / `sqlite` / `postgres`); this module is what
/// remains: the durable control-command outbox
/// (`repos::ControlQueueRepo` + `repos::InMemoryControlQueueRepo`,
/// `pg::PgControlQueueRepo` behind the `postgres` feature), the
/// idempotency-cache store (`repos::IdempotencyStoreRepo`, consumed by
/// the API idempotency middleware), the webhook-activation store, and
/// the identity-row repository surface implemented by the Postgres glue
/// in `pg`.
///
/// `pg::PgControlQueueRepo` is only present under the `postgres` feature,
/// so it is referenced with plain backticks (not an intra-doc link) to
/// keep default-feature rustdoc clean.
pub mod repos;
/// Database row types.
pub mod rows;
/// SQLite adapter implementing the `nebula-storage-port` contract
/// (dev / edge single-writer; spec §5 SQLite parity boundary).
#[cfg(feature = "sqlite")]
pub mod sqlite;

#[cfg(test)]
pub mod test_support;

pub use error::StorageError;
pub use format::StorageFormat;
pub use inmem::{
    InMemoryCheckpointStore, InMemoryControlQueue, InMemoryExecutionStore,
    InMemoryIdempotencyGuard, InMemoryIdempotencyStore, InMemoryJournalReader,
    InMemoryNodeResultStore, InMemoryResumeProducer, InMemoryResumeTokenStore,
    InMemoryWebhookActivationStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
};
