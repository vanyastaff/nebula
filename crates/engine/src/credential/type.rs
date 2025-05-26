use serde::{Deserialize, Serialize};
use strum_macros::AsRefStr;

/// Enumeration of standard credential types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, AsRefStr)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    /// API Key authentication
    ApiKey,

    /// Basic authentication (username/password)
    BasicAuth,

    /// OAuth2 authentication
    OAuth2,

    /// Bearer token authentication
    Bearer,

    /// Custom credential type with an identifier
    Custom(String),
}
