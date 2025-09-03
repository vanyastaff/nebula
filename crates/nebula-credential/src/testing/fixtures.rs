//! Test fixtures and data generators

use crate::core::*;
use crate::prelude::Credential;
use async_trait::async_trait;
use std::time::{Duration, SystemTime};

/// Create a test access token
pub fn test_token() -> AccessToken {
    AccessToken {
        token: SecureString::new("test-token-12345"),
        token_type: TokenType::Bearer,
        issued_at: SystemTime::now(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
        scopes: Some(vec!["read".to_string(), "write".to_string()]),
        claims: Default::default(),
    }
}

/// Create an expired test token
pub fn expired_token() -> AccessToken {
    AccessToken {
        token: SecureString::new("expired-token"),
        token_type: TokenType::Bearer,
        issued_at: SystemTime::now() - Duration::from_secs(7200),
        expires_at: Some(SystemTime::now() - Duration::from_secs(3600)),
        scopes: None,
        claims: Default::default(),
    }
}

/// Create an API key token (no expiry)
pub fn api_key_token() -> AccessToken {
    AccessToken {
        token: SecureString::new("sk-test-api-key"),
        token_type: TokenType::ApiKey,
        issued_at: SystemTime::now(),
        expires_at: None,
        scopes: None,
        claims: Default::default(),
    }
}

/// Test credential for testing
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestCredentialInput {
    pub value: String,
    pub should_fail: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestCredentialState {
    pub value: SecureString,
    pub refresh_count: u32,
    pub created_at: SystemTime,
}

impl CredentialState for TestCredentialState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "test_credential";
}

pub struct TestCredential {
    pub fail_on_refresh: bool,
    pub refresh_delay: Option<Duration>,
}

impl Default for TestCredential {
    fn default() -> Self {
        Self { fail_on_refresh: false, refresh_delay: None }
    }
}

#[async_trait]
impl Credential for TestCredential {
    type Input = TestCredentialInput;
    type State = TestCredentialState;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            id: "test_credential",
            name: "Test Credential",
            description: "Credential for testing",
            supports_refresh: true,
            requires_interaction: false,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<(Self::State, Option<AccessToken>)> {
        if input.should_fail {
            return Err(CredentialError::invalid_input("value", "configured to fail"));
        }

        let state = TestCredentialState {
            value: SecureString::new(&input.value),
            refresh_count: 0,
            created_at: SystemTime::now(),
        };

        Ok((state, Some(test_token())))
    }

    async fn refresh(
        &self,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<AccessToken> {
        if let Some(delay) = self.refresh_delay {
            tokio::time::sleep(delay).await;
        }

        if self.fail_on_refresh {
            return Err(CredentialError::RefreshFailed {
                id: "test".into(),
                reason: "configured to fail".into(),
            });
        }

        state.refresh_count += 1;
        Ok(test_token())
    }
}
