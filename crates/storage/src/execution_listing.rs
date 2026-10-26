//! Column codec for the execution listing projection shared by the SQL
//! backends (`0004_executions.sql`).
//!
//! Decoding fails closed: a status outside the closed set, a key outside the
//! representable range, or a timestamp that is not RFC 3339 is
//! [`StorageError::Corrupt`], never a guessed value.

#[cfg(feature = "sqlite")]
use nebula_storage_port::MicrosInstant;
use nebula_storage_port::{ExecutionListingStatus, StorageError};

/// Decode a stored status.
pub(crate) fn decode_status(stored: &str) -> Result<ExecutionListingStatus, StorageError> {
    stored
        .parse()
        .map_err(|_| StorageError::Corrupt("execution status column holds an unknown value".into()))
}

/// Decode an instant SQLite stores as integer microseconds.
#[cfg(feature = "sqlite")]
pub(crate) fn decode_sort_key(micros: i64) -> Result<MicrosInstant, StorageError> {
    MicrosInstant::from_micros(micros)
        .ok_or_else(|| StorageError::Corrupt("execution instant is out of range".into()))
}

/// SQL list of the stored names of every non-terminal status, for a
/// `status IN (...)` test against the row's current status.
pub(crate) const LIVE_STATUS_NAMES_SQL: &str = "'created', 'running', 'paused', 'cancelling'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_status_names_are_exactly_the_non_terminal_statuses() {
        let expected = ExecutionListingStatus::ALL
            .into_iter()
            .filter(|status| !status.is_terminal())
            .map(|status| format!("'{}'", status.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(LIVE_STATUS_NAMES_SQL, expected);
    }
}
