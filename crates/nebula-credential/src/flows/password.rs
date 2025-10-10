//! Username/Password credential flow (simple password-based auth)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core::{
    CredentialContext, CredentialError, CredentialState, SecureString,
    result::{CredentialFlow, InitializeResult},
};

/// Input for password-based authentication
#[derive(Clone, Serialize, Deserialize)]
pub struct PasswordInput {
    pub username: String,
    pub password: String,
}

/// State for password credential
#[derive(Clone, Serialize, Deserialize)]
pub struct PasswordState {
    pub username: String,
    pub password: SecureString,
}

impl CredentialState for PasswordState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "password";
}

/// Password flow implementation
pub struct PasswordFlow;

#[async_trait]
impl CredentialFlow for PasswordFlow {
    type Input = PasswordInput;
    type State = PasswordState;

    fn flow_name(&self) -> &'static str {
        "password"
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

        Ok(InitializeResult::Complete(PasswordState {
            username: input.username.clone(),
            password: SecureString::new(&input.password),
        }))
    }
}

/// Type alias for convenience
pub type PasswordCredential = crate::core::adapter::FlowCredential<PasswordFlow>;

impl PasswordCredential {
    /// Create a new Password credential
    #[must_use] 
    pub fn create() -> Self {
        Self::from_flow(PasswordFlow)
    }
}

impl Default for PasswordCredential {
    fn default() -> Self {
        Self::create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_password_flow() {
        let flow = PasswordFlow;
        let mut ctx = CredentialContext::new();

        let input = PasswordInput {
            username: "admin".to_string(),
            password: "secret123".to_string(),
        };

        let result = flow.execute(&input, &mut ctx).await.unwrap();

        match result {
            InitializeResult::Complete(state) => {
                assert_eq!(state.username, "admin");
                assert_eq!(state.password.expose(), "secret123");
            }
            _ => panic!("Expected Complete result"),
        }
    }

    #[tokio::test]
    async fn test_password_empty_username() {
        let flow = PasswordFlow;
        let mut ctx = CredentialContext::new();

        let input = PasswordInput {
            username: String::new(),
            password: "pass".to_string(),
        };

        let result = flow.execute(&input, &mut ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_password_empty_password() {
        let flow = PasswordFlow;
        let mut ctx = CredentialContext::new();

        let input = PasswordInput {
            username: "user".to_string(),
            password: String::new(),
        };

        let result = flow.execute(&input, &mut ctx).await;
        assert!(result.is_err());
    }
}
