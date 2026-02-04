//! Bearer Token flow (static token)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::{
    CredentialContext, CredentialError, CredentialState, SecretString,
    result::{CredentialFlow, InitializeResult},
};

/// Input for Bearer Token
#[derive(Clone, Serialize, Deserialize)]
pub struct BearerTokenInput {
    pub token: String,
}

/// State for Bearer Token credential
#[derive(Clone, Serialize, Deserialize)]
pub struct BearerTokenState {
    pub token: SecretString,
}

impl CredentialState for BearerTokenState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "bearer_token";
}

/// Bearer Token flow implementation
pub struct BearerTokenFlow;

#[async_trait]
impl CredentialFlow for BearerTokenFlow {
    type Input = BearerTokenInput;
    type State = BearerTokenState;

    fn flow_name(&self) -> &'static str {
        "bearer_token"
    }

    fn requires_interaction(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        if input.token.is_empty() {
            return Err(CredentialError::invalid_input(
                "token",
                "Bearer token cannot be empty",
            ));
        }

        Ok(InitializeResult::Complete(BearerTokenState {
            token: SecretString::new(&input.token),
        }))
    }
}

/// Type alias for convenience
pub type BearerTokenCredential = crate::core::adapter::FlowCredential<BearerTokenFlow>;

impl BearerTokenCredential {
    /// Create a new Bearer Token credential
    #[must_use]
    pub fn create() -> Self {
        Self::from_flow(BearerTokenFlow)
    }
}

impl Default for BearerTokenCredential {
    fn default() -> Self {
        Self::create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bearer_token_flow() {
        let flow = BearerTokenFlow;
        let mut ctx = CredentialContext::new();

        let input = BearerTokenInput {
            token: "my_secret_token_12345".to_string(),
        };

        let result = flow.execute(&input, &mut ctx).await.unwrap();

        match result {
            InitializeResult::Complete(state) => {
                assert_eq!(state.token.expose(), "my_secret_token_12345");
            }
            _ => panic!("Expected Complete result"),
        }
    }

    #[tokio::test]
    async fn test_bearer_token_empty_validation() {
        let flow = BearerTokenFlow;
        let mut ctx = CredentialContext::new();

        let input = BearerTokenInput {
            token: String::new(),
        };

        let result = flow.execute(&input, &mut ctx).await;
        assert!(result.is_err());
    }
}
