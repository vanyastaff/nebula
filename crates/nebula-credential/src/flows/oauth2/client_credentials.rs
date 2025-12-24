//! `OAuth2` Client Credentials flow

use async_trait::async_trait;
use nebula_log::prelude::{debug, error, info};
use serde::{Deserialize, Serialize};

use crate::core::{
    CredentialContext, CredentialError,
    result::{CredentialFlow, InitializeResult},
};

use super::common::{OAuth2State, TokenResponse};

/// Maximum length for error response body to log (prevents log flooding)
const MAX_ERROR_BODY_LOG_LENGTH: usize = 500;

/// Sanitize response body for logging - truncate and remove potential secrets
fn sanitize_response_for_logging(body: &str) -> String {
    let truncated = if body.len() > MAX_ERROR_BODY_LOG_LENGTH {
        format!(
            "{}... [truncated, {} total bytes]",
            &body[..MAX_ERROR_BODY_LOG_LENGTH],
            body.len()
        )
    } else {
        body.to_string()
    };

    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&truncated) {
        for field in [
            "access_token",
            "refresh_token",
            "id_token",
            "token",
            "secret",
            "password",
        ] {
            if json.get(field).is_some() {
                json[field] = serde_json::json!("[REDACTED]");
            }
        }
        json.to_string()
    } else {
        truncated
    }
}

/// Input for `OAuth2` Client Credentials flow
#[derive(Clone, Serialize, Deserialize)]
pub struct ClientCredentialsInput {
    pub client_id: String,
    pub client_secret: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// `OAuth2` Client Credentials flow implementation
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

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            error!(error = %e, "Failed to read token response body");
            CredentialError::NetworkFailed(e.to_string())
        })?;

        if !status.is_success() {
            let sanitized_body = sanitize_response_for_logging(&body);
            error!(
                status = %status,
                body = %sanitized_body,
                "Token request failed"
            );
            return Err(CredentialError::AuthenticationFailed {
                reason: format!("HTTP {status}"),
            });
        }

        let token: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            let sanitized_body = sanitize_response_for_logging(&body);
            error!(
                error = %e,
                body = %sanitized_body,
                "Failed to parse token response"
            );
            CredentialError::NetworkFailed(format!("Failed to parse token response: {e}"))
        })?;

        info!("OAuth2 client credentials flow completed successfully");

        Ok(InitializeResult::Complete(
            OAuth2State::from_token_response(token),
        ))
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
    /// Create a new `OAuth2` client credentials credential
    #[must_use]
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
