use nebula_core::CredentialKey;
use serde::{Deserialize, Serialize};

/// Metadata about a credential type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    /// Unique identifier for this credential type
    pub key: CredentialKey,

    /// Human-readable name
    pub name: String,

    /// Description of the credential
    pub description: String,

    /// Whether this credential supports refresh
    pub supports_refresh: bool,

    /// Whether user interaction is required
    pub requires_interaction: bool,
}
