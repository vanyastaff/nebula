//! Secret-free storage diagnostics shared by server-owned adapters.

use nebula_storage_port::StorageError;

/// Stable, payload-free category for logs at storage composition boundaries.
pub(crate) const fn storage_error_category(error: &StorageError) -> &'static str {
    match error {
        StorageError::NotFound { .. } => "not_found",
        StorageError::Conflict { .. } => "conflict",
        StorageError::Duplicate { .. } => "duplicate",
        StorageError::LeaseUnavailable { .. } => "lease_unavailable",
        StorageError::FencedOut { .. } => "fenced_out",
        StorageError::Timeout { .. } => "timeout",
        StorageError::UnknownSchemaVersion { .. } => "unknown_schema_version",
        StorageError::ScopeViolation { .. } => "scope_violation",
        StorageError::Serialization(_) => "serialization",
        StorageError::Connection(_) => "connection",
        StorageError::AcknowledgementUnknown { .. } => "acknowledgement_unknown",
        StorageError::Configuration(_) => "configuration",
        StorageError::Internal(_) => "internal",
        _ => "unknown",
    }
}
