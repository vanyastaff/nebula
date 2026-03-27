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

impl nebula_error::Classify for StorageError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::NotFound => nebula_error::ErrorCategory::NotFound,
            Self::Serialization(_) => nebula_error::ErrorCategory::Internal,
            Self::Backend(_) => nebula_error::ErrorCategory::External,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::NotFound => nebula_error::ErrorCode::new("STORAGE_NOT_FOUND"),
            Self::Serialization(_) => nebula_error::ErrorCode::new("STORAGE_SERIALIZATION"),
            Self::Backend(_) => nebula_error::ErrorCode::new("STORAGE_BACKEND"),
        }
    }
}
