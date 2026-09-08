//! Opaque identities used by the durable remote-operation protocol.

use core::fmt;

/// Identity retained for one prepared remote operation across retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId([u8; 16]);

impl OperationId {
    /// Reconstruct an identity from its durable representation.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Durable identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl From<[u8; 16]> for OperationId {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl From<OperationId> for [u8; 16] {
    fn from(operation_id: OperationId) -> Self {
        operation_id.0
    }
}

/// Identity of one storage-authorized invocation or read-only query.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct OperationCallId([u8; 16]);

impl OperationCallId {
    /// Reconstruct an identity from its durable representation.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Durable identity bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for OperationCallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl From<[u8; 16]> for OperationCallId {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl From<OperationCallId> for [u8; 16] {
    fn from(call_id: OperationCallId) -> Self {
        call_id.0
    }
}
