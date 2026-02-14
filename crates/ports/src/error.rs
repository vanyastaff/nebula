//! Error types for port operations.
//!
//! Every port method returns `Result<_, PortsError>`. Backend drivers map
//! their internal errors into these variants so the engine can make
//! retry/fallback decisions without knowing the concrete backend.

use std::time::Duration;

/// Error type for all port operations.
///
/// Distinguishes retryable failures (connection, timeout) from permanent
/// ones (not found, conflict, serialization) so the engine can apply
/// retry policies without inspecting error messages.
#[derive(Debug, thiserror::Error)]
pub enum PortsError {
    /// Entity not found.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Kind of entity (e.g. "Workflow", "Execution").
        entity: String,
        /// Identifier that was looked up.
        id: String,
    },

    /// Optimistic concurrency conflict.
    #[error("{entity} {id}: expected version {expected_version}, got {actual_version}")]
    Conflict {
        /// Kind of entity.
        entity: String,
        /// Identifier of the conflicting entity.
        id: String,
        /// Version the caller expected.
        expected_version: u64,
        /// Version currently stored.
        actual_version: u64,
    },

    /// Backend connection failure.
    #[error("connection error: {0}")]
    Connection(String),

    /// Serialization or deserialization failure.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Operation exceeded its timeout.
    #[error("timeout: {operation} after {duration:?}")]
    Timeout {
        /// Name of the operation that timed out.
        operation: String,
        /// How long was waited before giving up.
        duration: Duration,
    },

    /// Execution lease is held by another worker.
    #[error("lease unavailable for execution {execution_id}")]
    LeaseUnavailable {
        /// The execution whose lease could not be acquired.
        execution_id: String,
    },

    /// Catch-all internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl PortsError {
    /// Convenience constructor for [`PortsError::NotFound`].
    pub fn not_found(entity: impl Into<String>, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.into(),
        }
    }

    /// Convenience constructor for [`PortsError::Conflict`].
    pub fn conflict(
        entity: impl Into<String>,
        id: impl Into<String>,
        expected: u64,
        actual: u64,
    ) -> Self {
        Self::Conflict {
            entity: entity.into(),
            id: id.into(),
            expected_version: expected,
            actual_version: actual,
        }
    }

    /// Convenience constructor for [`PortsError::Timeout`].
    pub fn timeout(operation: impl Into<String>, duration: Duration) -> Self {
        Self::Timeout {
            operation: operation.into(),
            duration,
        }
    }

    /// Returns `true` for transient errors that the engine may retry.
    ///
    /// Currently [`Connection`](Self::Connection) and [`Timeout`](Self::Timeout).
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Connection(_) | Self::Timeout { .. })
    }
}

impl From<serde_json::Error> for PortsError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ── Construction ────────────────────────────────────────────────────

    #[test]
    fn not_found_convenience() {
        let err = PortsError::not_found("Workflow", "abc-123");
        match &err {
            PortsError::NotFound { entity, id } => {
                assert_eq!(entity, "Workflow");
                assert_eq!(id, "abc-123");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn conflict_convenience() {
        let err = PortsError::conflict("Execution", "exec-1", 3, 5);
        match &err {
            PortsError::Conflict {
                entity,
                id,
                expected_version,
                actual_version,
            } => {
                assert_eq!(entity, "Execution");
                assert_eq!(id, "exec-1");
                assert_eq!(*expected_version, 3);
                assert_eq!(*actual_version, 5);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn timeout_convenience() {
        let dur = Duration::from_secs(5);
        let err = PortsError::timeout("save_state", dur);
        match &err {
            PortsError::Timeout {
                operation,
                duration,
            } => {
                assert_eq!(operation, "save_state");
                assert_eq!(*duration, dur);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    // ── is_retryable ────────────────────────────────────────────────────

    #[test]
    fn connection_is_retryable() {
        assert!(PortsError::Connection("refused".into()).is_retryable());
    }

    #[test]
    fn timeout_is_retryable() {
        assert!(PortsError::timeout("op", Duration::from_secs(1)).is_retryable());
    }

    #[test]
    fn not_found_is_not_retryable() {
        assert!(!PortsError::not_found("X", "1").is_retryable());
    }

    #[test]
    fn conflict_is_not_retryable() {
        assert!(!PortsError::conflict("X", "1", 0, 1).is_retryable());
    }

    #[test]
    fn serialization_is_not_retryable() {
        assert!(!PortsError::Serialization("bad json".into()).is_retryable());
    }

    #[test]
    fn lease_unavailable_is_not_retryable() {
        assert!(
            !PortsError::LeaseUnavailable {
                execution_id: "e1".into()
            }
            .is_retryable()
        );
    }

    #[test]
    fn internal_is_not_retryable() {
        assert!(!PortsError::Internal("oops".into()).is_retryable());
    }

    // ── From<serde_json::Error> ─────────────────────────────────────────

    #[test]
    fn from_serde_json_error() {
        let bad_json = serde_json::from_str::<serde_json::Value>("not json");
        let serde_err = bad_json.unwrap_err();
        let ports_err: PortsError = serde_err.into();
        match &ports_err {
            PortsError::Serialization(msg) => {
                assert!(!msg.is_empty(), "message should not be empty");
            }
            other => panic!("expected Serialization, got {other:?}"),
        }
    }

    // ── Display ─────────────────────────────────────────────────────────

    #[test]
    fn display_not_found() {
        let err = PortsError::not_found("Workflow", "w-1");
        assert_eq!(err.to_string(), "Workflow not found: w-1");
    }

    #[test]
    fn display_conflict() {
        let err = PortsError::conflict("Execution", "e-1", 2, 4);
        assert_eq!(err.to_string(), "Execution e-1: expected version 2, got 4");
    }

    #[test]
    fn display_connection() {
        let err = PortsError::Connection("refused".into());
        assert_eq!(err.to_string(), "connection error: refused");
    }

    #[test]
    fn display_internal() {
        let err = PortsError::Internal("something broke".into());
        assert_eq!(err.to_string(), "internal error: something broke");
    }

    #[test]
    fn display_lease_unavailable() {
        let err = PortsError::LeaseUnavailable {
            execution_id: "exec-42".into(),
        };
        assert_eq!(err.to_string(), "lease unavailable for execution exec-42");
    }
}
