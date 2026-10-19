//! Backend-independent node-result rules.
//!
//! Every `NodeResultStore` adapter applies the same schema-version gate on
//! write and on read, and names its two per-node slots the same way, so the
//! in-memory reference model, SQLite and PostgreSQL cannot drift. Row
//! plumbing stays in each adapter.

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::{MAX_SUPPORTED_RESULT_SCHEMA_VERSION, NodeResultRecord};

/// Reject a record whose schema version this binary cannot decode.
///
/// A newer writer may have stored a shape this process does not understand;
/// surfacing it as a typed error keeps resume loud instead of silently
/// reconstructing a node from a misread payload. A save of such a record is
/// refused the same way, so nothing this binary writes is unreadable to it.
pub(crate) fn guard_schema(record: &NodeResultRecord) -> Result<(), StorageError> {
    if record.schema_version > MAX_SUPPORTED_RESULT_SCHEMA_VERSION {
        return Err(StorageError::UnknownSchemaVersion {
            found: record.schema_version,
            max: MAX_SUPPORTED_RESULT_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// The two per-node slots of an execution.
///
/// `Output` holds the raw payload a node emitted; `Result` holds the full
/// typed result record idempotent replay reconstructs routing from. They are
/// separate rows (separate tables on the SQL backends) and never alias.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeSlot {
    /// `save_node_output` / `load_node_output` / `load_all_node_outputs`.
    Output,
    /// `save_node_result` / `load_node_result` / `load_all_results`.
    Result,
}

#[cfg(any(feature = "sqlite", feature = "postgres"))]
impl NodeSlot {
    /// Span label for the slot.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Result => "result",
        }
    }
}

/// Decode a stored `schema_version` column. A value outside `u32` is corrupt
/// data, never a truncated version.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_schema_version(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| {
        StorageError::Corrupt("column `schema_version` is outside the u32 range".into())
    })
}

/// Build a record read back from storage and apply [`guard_schema`] to it.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn stored_record(
    kind_tag: String,
    json: serde_json::Value,
    schema_version: i64,
) -> Result<NodeResultRecord, StorageError> {
    let record = NodeResultRecord {
        kind_tag,
        json,
        schema_version: decode_schema_version(schema_version)?,
    };
    guard_schema(&record)?;
    Ok(record)
}

#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod tests {
    use super::*;

    #[test]
    fn a_schema_version_outside_u32_is_corrupt() {
        assert!(matches!(
            decode_schema_version(-1),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            decode_schema_version(i64::from(u32::MAX) + 1),
            Err(StorageError::Corrupt(_))
        ));
        assert_eq!(decode_schema_version(1).ok(), Some(1));
    }

    #[test]
    fn a_newer_stored_record_fails_closed() {
        let newer = i64::from(MAX_SUPPORTED_RESULT_SCHEMA_VERSION) + 1;
        assert!(matches!(
            stored_record("Value".into(), serde_json::json!(1), newer),
            Err(StorageError::UnknownSchemaVersion { .. })
        ));
    }
}
