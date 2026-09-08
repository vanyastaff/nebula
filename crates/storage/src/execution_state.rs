//! Size policy for opaque execution-state rows.

use std::io::{self, Write};

use nebula_storage_port::StorageError;

/// Absolute serialized ceiling for one execution aggregate row.
///
/// Checkpoint evidence has a stricter engine-owned ceiling. This storage
/// ceiling also bounds the surrounding inputs and metadata before a backend
/// returns JSON to a caller.
pub(crate) const MAX_PERSISTED_EXECUTION_STATE_BYTES: i64 = 64 * 1024 * 1024;

struct StateSizeCounter {
    bytes: i64,
    limit: i64,
}

impl StateSizeCounter {
    const fn new(limit: i64) -> Self {
        Self { bytes: 0, limit }
    }
}

impl Write for StateSizeCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let buffer_bytes = i64::try_from(buffer.len())
            .map_err(|_| io::Error::from(io::ErrorKind::FileTooLarge))?;
        self.bytes = self
            .bytes
            .checked_add(buffer_bytes)
            .filter(|bytes| *bytes <= self.limit)
            .ok_or_else(|| io::Error::from(io::ErrorKind::FileTooLarge))?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn ensure_execution_state_size(state: &serde_json::Value) -> Result<(), StorageError> {
    ensure_execution_state_size_with_limit(state, MAX_PERSISTED_EXECUTION_STATE_BYTES)
}

fn ensure_execution_state_size_with_limit(
    state: &serde_json::Value,
    limit: i64,
) -> Result<(), StorageError> {
    let mut counter = StateSizeCounter::new(limit);
    serde_json::to_writer(&mut counter, state).map_err(|_| oversized_execution_state())
}

pub(crate) fn oversized_execution_state() -> StorageError {
    StorageError::Serialization(format!(
        "persisted execution state exceeds {MAX_PERSISTED_EXECUTION_STATE_BYTES} bytes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_size_counter_rejects_before_serializing_past_the_limit() {
        let state = serde_json::json!({"payload": "bounded"});

        assert!(ensure_execution_state_size_with_limit(&state, 8).is_err());
        assert!(ensure_execution_state_size_with_limit(&state, 32).is_ok());
    }
}
