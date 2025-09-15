use crate::core::SecureString;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// Token type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    /// `OAuth2` Bearer token
    Bearer,
    /// API Key
    ApiKey,
    /// Basic authentication
    Basic,
    /// AWS `SigV4`
    AwsSigV4,
    /// Custom type
    Custom,
}

/// Access token with metadata
#[derive(Clone, Serialize, Deserialize)]
pub struct AccessToken {
    /// The actual token value (encrypted in memory)
    pub token: SecureString,

    /// Type of token
    pub token_type: TokenType,

    /// When the token was issued
    pub issued_at: SystemTime,

    /// When the token expires (if applicable)
    pub expires_at: Option<SystemTime>,

    /// `OAuth2` scopes
    pub scopes: Option<Vec<String>>,

    /// Additional claims/metadata
    pub claims: HashMap<String, serde_json::Value>,
}

impl AccessToken {
    /// Calculate TTL for the token
    pub fn ttl(&self) -> Option<Duration> {
        self.expires_at
            .and_then(|exp| exp.duration_since(SystemTime::now()).ok())
    }

    /// Check if token is expired
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|exp| exp <= SystemTime::now())
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessToken")
            .field("token_type", &self.token_type)
            .field("issued_at", &self.issued_at)
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}
