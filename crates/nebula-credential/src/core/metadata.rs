use serde::{Deserialize, Serialize};

/// Metadata about a credential type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    /// Unique identifier for this credential type
    pub id: &'static str,

    /// Human-readable name
    pub name: &'static str,

    /// Description of the credential
    pub description: &'static str,

    /// Whether this credential supports refresh
    pub supports_refresh: bool,

    /// Whether user interaction is required
    pub requires_interaction: bool,
}
