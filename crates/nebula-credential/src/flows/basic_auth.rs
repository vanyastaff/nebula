//! HTTP Basic Authentication flow

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::{
    CredentialContext, CredentialError, CredentialState, SecretString,
    result::{CredentialFlow, InitializeResult},
};

/// Input for Basic Auth
#[derive(Clone, Serialize, Deserialize)]
pub struct BasicAuthInput {
    pub username: String,
    pub password: String,
}

/// State for Basic Auth credential
#[derive(Clone, Serialize, Deserialize)]
pub struct BasicAuthState {
    pub username: String,
    pub password: SecretString,
}

impl CredentialState for BasicAuthState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "basic_auth";
}

/// Basic Auth flow implementation
pub struct BasicAuthFlow;

#[async_trait]
impl CredentialFlow for BasicAuthFlow {
    type Input = BasicAuthInput;
    type State = BasicAuthState;

    fn flow_name(&self) -> &'static str {
        "basic_auth"
    }

    fn requires_interaction(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        if input.username.is_empty() {
            return Err(CredentialError::invalid_input(
                "username",
                "Username cannot be empty",
            ));
        }

        if input.password.is_empty() {
            return Err(CredentialError::invalid_input(
                "password",
                "Password cannot be empty",
            ));
        }

        Ok(InitializeResult::Complete(BasicAuthState {
            username: input.username.clone(),
            password: SecretString::new(&input.password),
        }))
    }
}

/// Type alias for convenience
pub type BasicAuthCredential = crate::core::adapter::FlowCredential<BasicAuthFlow>;

impl BasicAuthCredential {
    /// Create a new Basic Auth credential
    #[must_use]
    pub fn create() -> Self {
        Self::from_flow(BasicAuthFlow)
    }
}

impl Default for BasicAuthCredential {
    fn default() -> Self {
        Self::create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_auth_flow() {
        let flow = BasicAuthFlow;
        let mut ctx = CredentialContext::new();

        let input = BasicAuthInput {
            username: "user".to_string(),
            password: "pass".to_string(),
        };

        let result = flow.execute(&input, &mut ctx).await.unwrap();

        match result {
            InitializeResult::Complete(state) => {
                assert_eq!(state.username, "user");
                assert_eq!(state.password.expose(), "pass");
            }
            _ => panic!("Expected Complete result"),
        }
    }

    #[tokio::test]
    async fn test_basic_auth_empty_username() {
        let flow = BasicAuthFlow;
        let mut ctx = CredentialContext::new();

        let input = BasicAuthInput {
            username: String::new(),
            password: "pass".to_string(),
        };

        let result = flow.execute(&input, &mut ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_basic_auth_empty_password() {
        let flow = BasicAuthFlow;
        let mut ctx = CredentialContext::new();

        let input = BasicAuthInput {
            username: "user".to_string(),
            password: String::new(),
        };

        let result = flow.execute(&input, &mut ctx).await;
        assert!(result.is_err());
    }
}
