use crate::core::{AccessToken, CredentialError, TokenType};
use super::ClientAuthenticator;
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
            return Err(CredentialError::invalid_config("HttpBearer requires a Bearer token"));
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
        Self {
            header_name: header_name.into(),
        }
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
            return Err(CredentialError::invalid_config("ApiKeyHeader requires an API key"));
        }

        let key_value = token.token.with_exposed(|s| s.to_string());

        Ok(builder.header(&self.header_name, key_value))
    }
}