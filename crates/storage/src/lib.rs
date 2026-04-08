//! # Nebula Storage
//!
//! Абстракция над различными системами хранения данных (Infrastructure Layer).
//!
//! По документации: [nebula-architecture-final](../../../docs/nebula-architecture-final.md).
//!
//! **Поддерживаемые backends:**
//! - In-memory — разработка и тесты (встроен)
//! - PostgreSQL — опционально, feature `postgres`
//! - Redis — опционально, feature `redis`
//! - S3/MinIO — опционально, feature `s3`
//! - Local filesystem — планируется

#![warn(missing_docs)]
#![warn(clippy::all)]

mod backend;
mod error;
mod execution_repo;
/// Serialization format abstraction (JSON / MessagePack).
pub mod format;
mod workflow_repo;

pub use backend::{MemoryStorage, MemoryStorageTyped};
#[cfg(feature = "postgres")]
pub use backend::{PgExecutionRepo, PgWorkflowRepo, PostgresStorage, PostgresStorageConfig};
pub use error::StorageError;
pub use execution_repo::{ExecutionRepo, ExecutionRepoError, InMemoryExecutionRepo};
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
