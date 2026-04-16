//! ULID to byte-array encoding for storage.
//!
//! Domain IDs (`ExecutionId`, `WorkflowId`, etc.) are prefixed ULID newtypes.
//! Storage stores them as 16-byte `BYTEA`/`BLOB`. This module handles conversion.

use crate::error::StorageError;

/// Encode a 16-byte ULID value to bytes for storage.
pub fn id_to_bytes(id: &[u8; 16]) -> Vec<u8> {
    id.to_vec()
}

/// Decode bytes from storage back to a 16-byte ULID.
///
/// # Errors
///
/// Returns [`StorageError::Serialization`] if `bytes` is not exactly 16 bytes.
pub fn bytes_to_id(bytes: &[u8]) -> Result<[u8; 16], StorageError> {
    <[u8; 16]>::try_from(bytes).map_err(|_| {
        StorageError::Serialization(format!("expected 16-byte ID, got {} bytes", bytes.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let original: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let encoded = id_to_bytes(&original);
        let decoded = bytes_to_id(&encoded).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn all_zeros() {
        let id = [0u8; 16];
        let decoded = bytes_to_id(&id_to_bytes(&id)).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn too_short() {
        let bytes = [0u8; 8];
        let err = bytes_to_id(&bytes).unwrap_err();
        assert!(err.to_string().contains("expected 16-byte ID"));
    }

    #[test]
    fn too_long() {
        let bytes = [0u8; 20];
        let err = bytes_to_id(&bytes).unwrap_err();
        assert!(err.to_string().contains("expected 16-byte ID"));
    }

    #[test]
    fn empty() {
        let err = bytes_to_id(&[]).unwrap_err();
        assert!(err.to_string().contains("0 bytes"));
    }
}
