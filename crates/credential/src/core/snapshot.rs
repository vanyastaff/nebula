//! Point-in-time credential snapshot.

use serde::{Deserialize, Serialize};

use crate::core::CredentialMetadata;

/// A point-in-time snapshot of a stored credential.
///
/// Returned when an action or context requests a credential by ID. Contains
/// the credential kind, its serialized runtime state, and associated metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialSnapshot {
    /// The credential type key (e.g. `"api_key"`, `"oauth2"`).
    pub kind: String,
    /// Serialized credential state as a JSON value.
    pub state: serde_json::Value,
    /// Associated credential metadata.
    pub metadata: CredentialMetadata,
}
