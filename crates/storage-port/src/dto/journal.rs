//! Execution-journal entry DTO (spec-16 §11.5).
use serde::{Deserialize, Serialize};

/// One append-only journal entry.
///
/// `seq` is assigned by the backend on append (`None` when the caller hands
/// an entry to `commit` and lets the store stamp the sequence). `payload` is
/// opaque to the port.
// `payload` is `serde_json::Value`, which is not `Eq` (it can hold a
// float). `Eq` is therefore not derivable; the clippy hint is a false
// positive for any DTO carrying an opaque JSON payload.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JournalEntry {
    /// Backend-assigned sequence number (`None` until persisted).
    pub seq: Option<u64>,
    /// Opaque journal payload.
    pub payload: serde_json::Value,
}
