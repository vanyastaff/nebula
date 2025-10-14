//! API Key credential flow

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::{
    CredentialContext, CredentialError, CredentialState, SecureString,
    result::{CredentialFlow, InitializeResult},
};

/// Input for API key authentication
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyInput {
    pub api_key: String,
}

/// State for API key credential
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    pub api_key: SecureString,
}

impl CredentialState for ApiKeyState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "api_key";
}

/// API Key flow implementation
pub struct ApiKeyFlow;

#[async_trait]
impl CredentialFlow for ApiKeyFlow {
    type Input = ApiKeyInput;
    type State = ApiKeyState;

    fn flow_name(&self) -> &'static str {
        "api_key"
    }

    fn requires_interaction(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        if input.api_key.is_empty() {
            return Err(CredentialError::invalid_input(
                "api_key",
                "API key cannot be empty",
            ));
        }

        Ok(InitializeResult::Complete(ApiKeyState {
            api_key: SecureString::new(&input.api_key),
        }))
    }
}

/// Type alias for convenience
pub type ApiKeyCredential = crate::core::adapter::FlowCredential<ApiKeyFlow>;

impl ApiKeyCredential {
    /// Create a new API key credential
    #[must_use]
    pub fn create() -> Self {
        Self::from_flow(ApiKeyFlow)
    }
}

impl Default for ApiKeyCredential {
    fn default() -> Self {
        Self::create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_api_key_flow() {
        let flow = ApiKeyFlow;
        let mut ctx = CredentialContext::new();

        let input = ApiKeyInput {
            api_key: "sk_test_12345".to_string(),
        };

        let result = flow.execute(&input, &mut ctx).await.unwrap();

        match result {
            InitializeResult::Complete(state) => {
                assert_eq!(state.api_key.expose(), "sk_test_12345");
            }
            _ => panic!("Expected Complete result"),
        }
    }

    #[tokio::test]
    async fn test_api_key_empty_validation() {
        let flow = ApiKeyFlow;
        let mut ctx = CredentialContext::new();

        let input = ApiKeyInput {
            api_key: String::new(),
        };

        let result = flow.execute(&input, &mut ctx).await;
        assert!(result.is_err());
    }
}
