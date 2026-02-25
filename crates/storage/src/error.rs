//! Ошибки хранилища.

use thiserror::Error;

/// Ошибки операций storage.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Ключ не найден (для операций, требующих существования ключа).
    #[error("key not found")]
    NotFound,
    /// Ошибка сериализации/десериализации.
    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Внутренняя/backend ошибка.
    #[error("backend: {0}")]
    Backend(String),
}
