//! Ошибки хранилища.

use thiserror::Error;

/// Ошибки операций storage.
#[derive(Debug, Error, nebula_error::Classify)]
pub enum StorageError {
    /// Ключ не найден (для операций, требующих существования ключа).
    #[classify(category = "not_found", code = "STORAGE_NOT_FOUND")]
    #[error("key not found")]
    NotFound,
    /// Ошибка сериализации/десериализации.
    #[classify(category = "internal", code = "STORAGE_SERIALIZATION")]
    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Внутренняя/backend ошибка.
    #[classify(category = "external", code = "STORAGE_BACKEND")]
    #[error("backend: {0}")]
    Backend(String),
}
