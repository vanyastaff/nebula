//! OAuth2 Client Credentials flow

use async_trait::async_trait;
use nebula_log::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::{
    result::{CredentialFlow, InitializeResult},
    CredentialContext, CredentialError,
};

use super::common::{OAuth2State, TokenResponse};

/// Input for OAuth2 Client Credentials flow
#[derive(Clone, Serialize, Deserialize)]
pub struct ClientCredentialsInput {
    pub client_id: String,
    pub client_secret: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// OAuth2 Client Credentials flow implementation
pub struct ClientCredentialsFlow;

#[async_trait]
impl CredentialFlow for ClientCredentialsFlow {
    type Input = ClientCredentialsInput;
    type State = OAuth2State;

    fn flow_name(&self) -> &'static str {
        "oauth2_client_credentials"
    }

    fn requires_interaction(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        debug!(
            client_id = %input.client_id,
            endpoint = %input.token_endpoint,
            scopes = ?input.scopes,
            "Executing OAuth2 client credentials flow"
        );

        let response = ctx
            .http_client()
            .post(&input.token_endpoint)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", &input.client_id),
                ("client_secret", &input.client_secret),
                ("scope", &input.scopes.join(" ")),
            ])
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to send token request");
                CredentialError::NetworkFailed(e.to_string())
            })?;

        if !response.status().is_success() {
            error!(status = %response.status(), "Token request failed");
            return Err(CredentialError::AuthenticationFailed {
                reason: format!("HTTP {}", response.status()),
            });
        }

        let token: TokenResponse = response
            .json()
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to parse token response");
                CredentialError::NetworkFailed(e.to_string())
            })?;

        info!("OAuth2 client credentials flow completed successfully");

        Ok(InitializeResult::Complete(OAuth2State::from_token_response(token)))
    }

    async fn refresh(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        // For client credentials, we typically don't have refresh tokens
        // Instead, we should re-execute the flow with stored credentials
        // For now, return not supported
        Err(CredentialError::refresh_not_supported(
            self.flow_name().to_string(),
        ))
    }
}

/// Type alias for convenience
pub type OAuth2ClientCredentials = crate::core::adapter::FlowCredential<ClientCredentialsFlow>;

impl OAuth2ClientCredentials {
    /// Create a new OAuth2 client credentials credential
    pub fn create() -> Self {
        Self::from_flow(ClientCredentialsFlow)
    }
}

impl Default for OAuth2ClientCredentials {
    fn default() -> Self {
        Self::create()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_serialization() {
        let input = ClientCredentialsInput {
            client_id: "test_client".to_string(),
            client_secret: "test_secret".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
        };

        let json = serde_json::to_string(&input).unwrap();
        let deserialized: ClientCredentialsInput = serde_json::from_str(&json).unwrap();

        assert_eq!(input.client_id, deserialized.client_id);
        assert_eq!(input.scopes, deserialized.scopes);
    }
}
