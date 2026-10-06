//! Column codec for the execution listing projection shared by the SQL
//! backends (migration 0064).
//!
//! Decoding fails closed: a status outside the closed set or a key outside
//! the representable range is a [`StorageError::Serialization`], never a
//! guessed value.

use nebula_storage_port::{ExecutionListingStatus, MicrosInstant, StorageError};

/// Decode a stored status.
pub(crate) fn decode_status(stored: &str) -> Result<ExecutionListingStatus, StorageError> {
    stored.parse().map_err(|_| {
        StorageError::Serialization("execution status column holds an unknown value".into())
    })
}

/// Decode the `created_at_us` sort key.
pub(crate) fn decode_sort_key(micros: i64) -> Result<MicrosInstant, StorageError> {
    MicrosInstant::from_micros(micros)
        .ok_or_else(|| StorageError::Serialization("execution creation key is out of range".into()))
}

/// Decode an RFC 3339 timestamp stored as text (SQLite).
#[cfg(feature = "sqlite")]
pub(crate) fn decode_text_instant(stored: &str) -> Result<MicrosInstant, StorageError> {
    chrono::DateTime::parse_from_rfc3339(stored)
        .map(|instant| MicrosInstant::floor(instant.with_timezone(&chrono::Utc)))
        .map_err(|_| StorageError::Serialization("execution timestamp is not RFC 3339".into()))
}

/// Encode an instant as the RFC 3339 text SQLite stores.
#[cfg(feature = "sqlite")]
pub(crate) fn encode_text_instant(instant: MicrosInstant) -> String {
    instant
        .to_datetime()
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}
