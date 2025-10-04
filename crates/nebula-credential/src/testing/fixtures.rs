//! Test fixtures and data generators

use crate::core::{
    AccessToken, CredentialContext, CredentialError, CredentialMetadata, CredentialState, Result,
    SecureString, TokenType,
};
use crate::traits::Credential;
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
        Self {
            fail_on_refresh: false,
            refresh_delay: None,
        }
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
            return Err(CredentialError::invalid_input(
                "value",
                "configured to fail",
            ));
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

/// Factory for creating test credentials
pub struct TestCredentialFactory {
    credential: TestCredential,
}

impl TestCredentialFactory {
    /// Create new factory with default test credential
    pub fn new() -> Self {
        Self {
            credential: TestCredential::default(),
        }
    }

    /// Create factory with custom credential behavior
    pub fn with_credential(credential: TestCredential) -> Self {
        Self { credential }
    }
}

impl Default for TestCredentialFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl crate::registry::CredentialFactory for TestCredentialFactory {
    fn type_name(&self) -> &'static str {
        "test_credential"
    }

    async fn create_and_init(
        &self,
        input_json: serde_json::Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, Option<AccessToken>)> {
        let input: TestCredentialInput = serde_json::from_value(input_json)
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        let (state, token) = self.credential.initialize(&input, cx).await?;
        Ok((Box::new(state), token))
    }

    async fn refresh(
        &self,
        state_json: serde_json::Value,
        cx: &mut CredentialContext,
    ) -> Result<(Box<dyn erased_serde::Serialize>, AccessToken)> {
        let mut state: TestCredentialState = serde_json::from_value(state_json)
            .map_err(|e| CredentialError::DeserializationFailed(e.to_string()))?;

        let token = self.credential.refresh(&mut state, cx).await?;
        Ok((Box::new(state), token))
    }
}
