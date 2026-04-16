//! Unified storage error types.
//!
//! One error enum for all repository operations. Domain repos return
//! `Result<T, StorageError>` — no per-repo error types to juggle.

use std::time::Duration;

use thiserror::Error;

/// Errors returned by any storage repository operation.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Entity not found.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity kind (`"execution"`, `"workflow"`, etc.).
        entity: &'static str,
        /// Entity identifier (display form).
        id: String,
    },

    /// Optimistic concurrency (CAS) version mismatch.
    #[error("{entity} {id}: expected version {expected}, got {actual}")]
    Conflict {
        /// Entity kind.
        entity: &'static str,
        /// Entity identifier.
        id: String,
        /// Version the caller expected.
        expected: i64,
        /// Actual version in storage.
        actual: i64,
    },

    /// Unique constraint violation (slug, idempotency key, dedup, etc.).
    #[error("duplicate {entity}: {detail}")]
    Duplicate {
        /// Entity kind.
        entity: &'static str,
        /// Human-readable detail.
        detail: String,
    },

    /// Execution lease is held by another owner.
    #[error("lease unavailable for {entity} {id}")]
    LeaseUnavailable {
        /// Entity kind (usually `"execution"`).
        entity: &'static str,
        /// Entity identifier.
        id: String,
    },

    /// Operation timed out.
    #[error("timeout: {operation} after {duration:?}")]
    Timeout {
        /// Name of the operation.
        operation: String,
        /// Elapsed duration.
        duration: Duration,
    },

    /// Serialization or deserialization failure.
    #[error("serialization: {0}")]
    Serialization(String),

    /// Database connection or driver error.
    #[error("connection: {0}")]
    Connection(String),

    /// Invalid configuration (bad URL, missing param, etc.).
    #[error("configuration: {0}")]
    Configuration(String),

    /// Unexpected internal error.
    #[error("internal: {0}")]
    Internal(String),
}

impl StorageError {
    /// Shorthand for [`StorageError::NotFound`].
    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }

    /// Shorthand for [`StorageError::Conflict`].
    pub fn conflict(
        entity: &'static str,
        id: impl Into<String>,
        expected: i64,
        actual: i64,
    ) -> Self {
        Self::Conflict {
            entity,
            id: id.into(),
            expected,
            actual,
        }
    }

    /// Shorthand for [`StorageError::Duplicate`].
    pub fn duplicate(entity: &'static str, detail: impl Into<String>) -> Self {
        Self::Duplicate {
            entity,
            detail: detail.into(),
        }
    }

    /// Shorthand for [`StorageError::Timeout`].
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> Self {
        Self::Timeout {
            operation: operation.into(),
            duration,
        }
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}
