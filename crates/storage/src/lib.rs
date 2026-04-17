//! # Nebula Storage
//!
//! Persistence abstraction (Infrastructure layer).
//!
//! ## Two coexisting trait layers — status, per canon §11.6
//!
//! This crate currently exposes **two** trait layers for execution / workflow
//! persistence. The engine, API, and runtime consume the first; the second
//! is a roadmap design that is not yet wired end-to-end. Both are public
//! today so that downstream code can migrate incrementally.
//!
//! ### Layer 1 — `ExecutionRepo` / `WorkflowRepo` (top-level re-exports) — **implemented**
//!
//! The production path. State is stored as opaque `serde_json::Value` blobs
//! with typed ID keys and `u64` optimistic-CAS versions. Implementations:
//! [`InMemoryExecutionRepo`], [`InMemoryWorkflowRepo`], and (feature `postgres`)
//! `PgExecutionRepo`, `PgWorkflowRepo`, `PostgresStorage`.
//!
//! This is the layer the **knife scenario** (`docs/PRODUCT_CANON.md` §13)
//! exercises end-to-end. If you are writing an engine, handler, or test
//! today, depend on this layer.
//!
//! ### Layer 2 — [`repos`] (`repos::ExecutionRepo`, `repos::WorkflowRepo`, ...) — **planned / experimental**
//!
//! The spec-16 row model (see `docs/plans/2026-04-15-arch-specs/16-storage-schema.md`):
//! structured rows, mandatory multi-tenancy (`workspace_id` / `org_id`),
//! split `WorkflowRow` + `WorkflowVersionRow`, idempotency as a column on
//! `ExecutionNodeRow`, etc. Trait definitions only — **no in-memory or
//! Postgres implementations exist yet**, and the engine / API cannot
//! compile against these signatures without a broader refactor.
//!
//! **Exception — [`repos::ControlQueueRepo`] is implemented** via
//! [`repos::InMemoryControlQueueRepo`] and is already wired into the API
//! cancel path (`crates/api/src/handlers/execution.rs`). It is the only
//! new-layer contract consumers should depend on today.
//!
//! Adopting the rest of layer 2 requires a dedicated design + plan
//! ("Sprint E — adopt spec-16 row model" in
//! `docs/superpowers/specs/2026-04-16-workspace-health-audit.md`). Until
//! that lands, the new traits are `planned` per canon §11.6 — they should
//! not be presented as a current contract in docs or error messages.
//!
//! ## Supported backends (layer 1)
//!
//! - **In-memory** — development and tests (always available, no feature flag).
//! - **PostgreSQL** — optional, feature `postgres`.
//! - **Redis** — optional, feature `redis` (Storage KV only, not execution).
//! - **S3 / MinIO** — optional, feature `s3` (blob storage).
//! - **Local filesystem** — planned.

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
/// Only [`repos::ControlQueueRepo`] + [`repos::InMemoryControlQueueRepo`]
/// are implemented and actually consumed today (by the API cancel path).
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
    ExecutionRepo, ExecutionRepoError, InMemoryExecutionRepo, StatefulCheckpointRecord,
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
