//! # nebula-storage — Storage Port
//!
//! Persistence seam for the Nebula execution engine. Provides `ExecutionRepo`
//! and `WorkflowRepo` as the production persistence interfaces, backed by
//! SQLite (dev / test) or PostgreSQL (production).
//!
//! ## Layer 1 — production interfaces (use these today)
//!
//! Top-level re-exports: `ExecutionRepo`, `WorkflowRepo`, `InMemoryExecutionRepo`,
//! `InMemoryWorkflowRepo`, `NodeResultRecord`, `MAX_SUPPORTED_RESULT_SCHEMA_VERSION`.
//! Feature `postgres` adds `PgExecutionRepo`, `PgWorkflowRepo`, `PostgresStorage`.
//!
//! This is the layer the knife scenario (`docs/PRODUCT_CANON.md` §13) exercises
//! end-to-end.
//!
//! ## Layer 2 — `repos` module — planned / experimental (canon §11.6)
//!
//! Spec-16 row model: structured rows, mandatory multi-tenancy, split workflow
//! versioning. Trait definitions only — no implementations exist yet; the engine
//! cannot compile against these without a broader refactor.
//!
//! **Exception:** `repos::ControlQueueRepo` is a production trait consumed
//! by `nebula_engine::ControlConsumer`. Two backings live in this crate:
//! `repos::InMemoryControlQueueRepo` is the currently-wired runtime
//! (tests, local, `simple_server` example), and `pg::PgControlQueueRepo`
//! is available behind the `postgres` feature for multi-process /
//! restart-tolerant deployments (`FOR UPDATE SKIP LOCKED` per ADR-0008
//! §1). No composition root selects the Postgres backing by default
//! yet — a future `apps/server` (ADR-0008 follow-up) wires it in.
//!
//! ## Canon
//!
//! - §11.1 CAS transitions via `ExecutionRepo::transition`.
//! - §11.3 idempotency check-and-mark via `ExecutionRepo`.
//! - §11.5 journal (`append_journal`) and checkpoint (`save_stateful_checkpoint`).
//! - §12.2 outbox atomicity: `execution_control_queue` writes share the same operation as state
//!   transitions.
//! - §12.3 local path: SQLite is the default; `test_support` provides `sqlite_memory_*` helpers for
//!   in-process tests.
//! - ADR-0009 resume-persistence schema: `set_workflow_input` / `get_workflow_input` and
//!   `save_node_result` / `load_node_result` / `load_all_results` expose the seam; engine consumers
//!   (chips B2 / B3 / B4) wire the resume path.
//!
//! See `crates/storage/README.md` for the full durability matrix and
//! backend status table.

#![warn(missing_docs)]
#![warn(clippy::all)]

mod backend;
mod error;
mod execution_repo;
/// Serialization format abstraction (JSON / MessagePack).
pub mod format;
/// Row-to-domain type conversion utilities.
pub mod mapping;
/// Postgres implementations of [`repos`] traits.
#[cfg(feature = "postgres")]
pub mod pg;
/// Connection pool configuration.
pub mod pool;
/// Repository trait API (spec-16 architecture) — **planned / experimental**, per canon §11.6.
///
/// Only [`repos::ControlQueueRepo`] has a production-wired consumer today
/// (`nebula_engine::ControlConsumer`). Two backings live in this crate:
/// [`repos::InMemoryControlQueueRepo`] is the currently selected runtime
/// (tests, local, `simple_server` example); `pg::PgControlQueueRepo` is
/// available behind the `postgres` feature for multi-process /
/// restart-tolerant deployments (`FOR UPDATE SKIP LOCKED` per ADR-0008
/// §1) but is not yet selected by any composition root — a future
/// `apps/server` binary (ADR-0008 follow-up) wires it in.
///
/// `pg::PgControlQueueRepo` is only present under the `postgres` feature,
/// so this reference is intentionally kept as plain backticks (not an
/// intra-doc link) to keep default-feature rustdoc clean.
///
/// The rest of the traits in this module are design placeholders with
/// no implementations yet — adopting them requires an engine + API
/// refactor tracked as "Sprint E — adopt spec-16 row model" in the
/// workspace health audit spec.
///
/// For execution / workflow persistence, use the top-level [`ExecutionRepo`]
/// and [`WorkflowRepo`] re-exports (layer 1) — they are the production
/// contract the knife scenario (canon §13) runs against.
pub mod repos;
/// Database row types.
pub mod rows;
mod workflow_repo;

#[cfg(test)]
pub mod test_support;

pub use backend::{MemoryStorage, MemoryStorageTyped};
#[cfg(feature = "postgres")]
pub use backend::{PgExecutionRepo, PgWorkflowRepo, PostgresStorage, PostgresStorageConfig};
pub use error::StorageError;
pub use execution_repo::{
    ExecutionRepo, ExecutionRepoError, InMemoryExecutionRepo, MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
    NodeResultRecord, StatefulCheckpointRecord,
};
pub use format::StorageFormat;
pub use storage::Storage;
pub use workflow_repo::{InMemoryWorkflowRepo, WorkflowRepo, WorkflowRepoError};

mod storage {
    use async_trait::async_trait;

    use crate::StorageError;

    /// Универсальный trait для хранилищ (key-value).
    ///
    /// Реализации: in-memory, Redis, Postgres, S3 — см. [nebula-architecture-final](https://github.com/vanyastaff/nebula/blob/main/docs/nebula-architecture-final.md#nebula-storage).
    #[async_trait]
    pub trait Storage: Send + Sync {
        /// Тип ключа (например `String`, `WorkflowId`).
        type Key: Send + Sync;
        /// Тип значения (сериализуемый или бинарный).
        type Value: Send + Sync;

        /// Получить значение по ключу.
        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, StorageError>;
        /// Записать значение по ключу.
        async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), StorageError>;
        /// Удалить ключ.
        async fn delete(&self, key: &Self::Key) -> Result<(), StorageError>;
        /// Проверить наличие ключа.
        async fn exists(&self, key: &Self::Key) -> Result<bool, StorageError>;
    }
}
