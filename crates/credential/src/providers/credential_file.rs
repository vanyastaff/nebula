//! Shared serialization format for credential persistence.
//!
//! Used by local filesystem and Postgres (KV) providers so that
//! the same JSON shape is stored regardless of backend.

use serde::{Deserialize, Serialize};

use crate::core::CredentialMetadata;
use crate::utils::EncryptedData;

/// Serialization format for stored credentials (file or KV value).
///
/// Stored as JSON with encryption and metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialFile {
    /// Format version for future migration support
    pub version: u32,

    /// Encrypted credential data
    pub encrypted_data: EncryptedData,

    /// Credential metadata
    pub metadata: CredentialMetadata,

    /// Salt used for encryption (for future key derivation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt: Option<Vec<u8>>,
}

/// Current format version
pub const CREDENTIAL_FILE_VERSION: u32 = 1;

impl CredentialFile {
    /// Create new credential file
    pub fn new(encrypted_data: EncryptedData, metadata: CredentialMetadata) -> Self {
        Self {
            version: CREDENTIAL_FILE_VERSION,
            encrypted_data,
            metadata,
            salt: None,
        }
    }

    /// Check if file needs migration
    pub fn needs_migration(&self) -> bool {
        self.version < CREDENTIAL_FILE_VERSION
    }
}
