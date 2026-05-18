//! Unified storage error.
//!
//! Every port operation returns [`StorageError`]. It is `#[non_exhaustive]`
//! so the adapter can grow new failure modes without a breaking change, and
//! every variant is fail-closed (no variant silently degrades to success).
use std::time::Duration;

/// Error returned by every port operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Entity absent (also returned on a deliberate cross-scope miss so the
    /// existence of another tenant's row never leaks).
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity name (static — safe to log).
        entity: &'static str,
        /// Opaque id of the missing entity.
        id: String,
    },
    /// Optimistic-CAS version mismatch.
    #[error("{entity} {id}: version conflict (expected {expected}, actual {actual})")]
    Conflict {
        /// Entity name.
        entity: &'static str,
        /// Entity id.
        id: String,
        /// Version the caller expected.
        expected: u64,
        /// Version actually persisted.
        actual: u64,
    },
    /// Unique-constraint / first-writer collision.
    #[error("{entity} duplicate: {detail}")]
    Duplicate {
        /// Entity name.
        entity: &'static str,
        /// Human-readable collision detail.
        detail: String,
    },
    /// Lease could not be acquired.
    #[error("{entity} {id}: lease unavailable")]
    LeaseUnavailable {
        /// Entity name.
        entity: &'static str,
        /// Entity id.
        id: String,
    },
    /// Caller's fencing token was superseded by a newer lease generation.
    #[error("{entity} {id}: fenced out")]
    FencedOut {
        /// Entity name.
        entity: &'static str,
        /// Entity id.
        id: String,
    },
    /// Operation exceeded its deadline.
    #[error("{operation} timed out after {duration:?}")]
    Timeout {
        /// Operation name.
        operation: String,
        /// Elapsed time before the deadline tripped.
        duration: Duration,
    },
    /// Persisted record carries a schema version this binary cannot decode.
    #[error("unknown schema version {found} (max supported {max})")]
    UnknownSchemaVersion {
        /// Schema version found on the persisted record.
        found: u32,
        /// Maximum schema version this binary supports.
        max: u32,
    },
    /// Cross-tenant access denial surfaced to audit (never leaks the row).
    #[error("{entity}: scope violation")]
    ScopeViolation {
        /// Entity name.
        entity: &'static str,
    },
    /// (De)serialization failure.
    #[error("serialization: {0}")]
    Serialization(String),
    /// Backend connectivity failure.
    #[error("connection: {0}")]
    Connection(String),
    /// Misconfiguration (fail-closed — never proceed on a misconfigured path).
    #[error("configuration: {0}")]
    Configuration(String),
    /// Unexpected internal invariant break.
    #[error("internal: {0}")]
    Internal(String),
}

impl StorageError {
    /// Construct a [`StorageError::NotFound`].
    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}
