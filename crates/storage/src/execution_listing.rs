//! Column codec for the execution listing projection shared by the SQL
//! backends (`0004_executions.sql`).
//!
//! Decoding fails closed: a status outside the closed set, a key outside the
//! representable range, or a timestamp that is not RFC 3339 is
//! [`StorageError::Corrupt`], never a guessed value.

use nebula_storage_port::{ExecutionListingStatus, MicrosInstant, StorageError};

/// Decode a stored status.
pub(crate) fn decode_status(stored: &str) -> Result<ExecutionListingStatus, StorageError> {
    stored
        .parse()
        .map_err(|_| StorageError::Corrupt("execution status column holds an unknown value".into()))
}

/// Decode an instant SQLite stores as integer microseconds.
pub(crate) fn decode_sort_key(micros: i64) -> Result<MicrosInstant, StorageError> {
    MicrosInstant::from_micros(micros)
        .ok_or_else(|| StorageError::Corrupt("execution instant is out of range".into()))
}
