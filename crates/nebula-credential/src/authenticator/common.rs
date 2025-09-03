use super::ClientAuthenticator;
use crate::core::{AccessToken, CredentialError, TokenType};
use async_trait::async_trait;

/// HTTP Bearer token authenticator
pub struct HttpBearer;

#[async_trait]
impl ClientAuthenticator for HttpBearer {
    type Target = http::request::Builder;
    type Output = http::request::Builder;

    async fn authenticate(
        &self,
        mut builder: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::Bearer) {
            return Err(CredentialError::InvalidConfiguration {
                reason: "HttpBearer requires a Bearer token".to_string(),
            });
        }

        let auth_value = token.token.with_exposed(|s| format!("Bearer {}", s));

        Ok(builder.header("Authorization", auth_value))
    }
}

/// API Key authenticator (header-based)
pub struct ApiKeyHeader {
    /// Header name to use
    pub header_name: String,
}

impl ApiKeyHeader {
    /// Create new API key authenticator
    pub fn new(header_name: impl Into<String>) -> Self {
        Self { header_name: header_name.into() }
    }
}

#[async_trait]
impl ClientAuthenticator for ApiKeyHeader {
    type Target = http::request::Builder;
    type Output = http::request::Builder;

    async fn authenticate(
        &self,
        mut builder: Self::Target,
        token: &AccessToken,
    ) -> Result<Self::Output, CredentialError> {
        if !matches!(token.token_type, TokenType::ApiKey) {
            return Err(CredentialError::InvalidConfiguration {
                reason: "ApiKeyHeader requires an API key".to_string(),
            });
        }

        let key_value = token.token.with_exposed(|s| s.to_string());

        Ok(builder.header(&self.header_name, key_value))
    }
}
