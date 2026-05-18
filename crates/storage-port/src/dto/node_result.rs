//! Node-result row DTO (ADR-0009).
use serde::{Deserialize, Serialize};

/// Maximum node-result schema version this binary can decode. A persisted
/// record with a higher version fails closed with
/// [`crate::StorageError::UnknownSchemaVersion`] rather than being silently
/// misinterpreted.
pub const MAX_SUPPORTED_RESULT_SCHEMA_VERSION: u32 = 1;

/// A persisted node result, port-local.
///
/// `kind_tag` is the discriminant the producer stamped (e.g. `"Value"`);
/// `json` is the opaque payload. The port deliberately does **not** depend on
/// `ActionResult` — decoding back into a typed result is the caller's job.
// guard-justified: `json` is `serde_json::Value`, which is not `Eq`
// (it can hold a float). `Eq` is therefore not derivable; the clippy
// hint is a false positive for any DTO carrying an opaque JSON payload.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeResultRecord {
    /// Producer-stamped result-kind discriminant.
    pub kind_tag: String,
    /// Opaque serialized result payload.
    pub json: serde_json::Value,
    /// Schema version of `json` (checked against
    /// [`MAX_SUPPORTED_RESULT_SCHEMA_VERSION`]).
    pub schema_version: u32,
}
