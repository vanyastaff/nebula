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

impl AccessToken {
    /// Create a new bearer token
    pub fn bearer(token: String) -> Self {
        Self {
            token: SecureString::new(token),
            token_type: TokenType::Bearer,
            issued_at: SystemTime::now(),
            expires_at: None,
            scopes: None,
            claims: HashMap::new(),
        }
    }

    /// Create a new API key token
    pub fn api_key(key: String) -> Self {
        Self {
            token: SecureString::new(key),
            token_type: TokenType::ApiKey,
            issued_at: SystemTime::now(),
            expires_at: None,
            scopes: None,
            claims: HashMap::new(),
        }
    }

    /// Create with expiration
    pub fn with_expiration(mut self, expires_at: SystemTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Create with scopes
    pub fn with_scopes(mut self, scopes: Vec<String>) -> Self {
        self.scopes = Some(scopes);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_token_bearer_creation() {
        let token = AccessToken::bearer("test-token-123".to_string());
        assert_eq!(token.token_type, TokenType::Bearer);
        assert!(token.expires_at.is_none());
        assert!(token.scopes.is_none());
    }

    #[test]
    fn test_access_token_api_key_creation() {
        let token = AccessToken::api_key("api-key-abc".to_string());
        assert_eq!(token.token_type, TokenType::ApiKey);
        assert!(token.expires_at.is_none());
    }

    #[test]
    fn test_access_token_with_expiration() {
        let future = SystemTime::now() + Duration::from_secs(3600);
        let token = AccessToken::bearer("test".to_string()).with_expiration(future);
        assert_eq!(token.expires_at, Some(future));
        assert!(!token.is_expired());
    }

    #[test]
    fn test_access_token_is_expired_when_past() {
        let past = SystemTime::now() - Duration::from_secs(3600);
        let token = AccessToken::bearer("test".to_string()).with_expiration(past);
        assert!(token.is_expired());
    }

    #[test]
    fn test_access_token_is_not_expired_when_no_expiration() {
        let token = AccessToken::bearer("test".to_string());
        assert!(!token.is_expired());
    }

    #[test]
    fn test_access_token_ttl_calculation() {
        let future = SystemTime::now() + Duration::from_secs(300);
        let token = AccessToken::bearer("test".to_string()).with_expiration(future);
        let ttl = token.ttl().expect("should have TTL");

        // TTL should be approximately 300 seconds (allow 1 second tolerance)
        assert!(ttl.as_secs() >= 299 && ttl.as_secs() <= 300);
    }

    #[test]
    fn test_access_token_ttl_none_when_no_expiration() {
        let token = AccessToken::bearer("test".to_string());
        assert!(token.ttl().is_none());
    }

    #[test]
    fn test_access_token_ttl_none_when_expired() {
        let past = SystemTime::now() - Duration::from_secs(10);
        let token = AccessToken::bearer("test".to_string()).with_expiration(past);
        assert!(token.ttl().is_none());
    }

    #[test]
    fn test_access_token_with_scopes() {
        let scopes = vec!["read".to_string(), "write".to_string()];
        let token = AccessToken::bearer("test".to_string()).with_scopes(scopes.clone());
        assert_eq!(token.scopes, Some(scopes));
    }

    #[test]
    fn test_access_token_debug_does_not_leak_token() {
        let token = AccessToken::bearer("super-secret-token".to_string());
        let debug_str = format!("{:?}", token);
        assert!(!debug_str.contains("super-secret"));
        assert!(debug_str.contains("AccessToken"));
    }

    #[test]
    fn test_access_token_serialization() {
        let token = AccessToken::bearer("test".to_string())
            .with_scopes(vec!["read".to_string()]);

        let json = serde_json::to_string(&token).expect("serialization should work");
        let deserialized: AccessToken =
            serde_json::from_str(&json).expect("deserialization should work");

        assert_eq!(token.token_type, deserialized.token_type);
        assert_eq!(token.scopes, deserialized.scopes);
    }

    #[test]
    fn test_token_type_variants() {
        assert_eq!(TokenType::Bearer, TokenType::Bearer);
        assert_ne!(TokenType::Bearer, TokenType::ApiKey);
        assert_ne!(TokenType::ApiKey, TokenType::Basic);
    }
}
