//! OAuth2 Authorization Code flow (interactive)

use async_trait::async_trait;
use nebula_log::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::core::{
    CredentialContext, CredentialError, CredentialKey, CredentialMetadata,
    result::{CredentialFlow, InitializeResult, InteractionRequest, PartialState, UserInput},
    unix_now,
};
use crate::traits::{Credential, InteractiveCredential};
use crate::utils::{generate_code_challenge, generate_pkce_verifier, generate_random_state};

use super::common::{OAuth2State, TokenResponse};

/// Input for OAuth2 Authorization Code flow
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthorizationCodeInput {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub use_pkce: bool,
}

/// OAuth2 Authorization Code flow implementation
pub struct AuthorizationCodeFlow;

#[async_trait]
impl CredentialFlow for AuthorizationCodeFlow {
    type Input = AuthorizationCodeInput;
    type State = OAuth2State;

    fn flow_name(&self) -> &'static str {
        "oauth2_authorization_code"
    }

    fn requires_interaction(&self) -> bool {
        true
    }

    async fn execute(
        &self,
        input: &Self::Input,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        debug!(
            client_id = %input.client_id,
            use_pkce = input.use_pkce,
            scopes = ?input.scopes,
            "Starting OAuth2 authorization code flow"
        );

        let state_param = generate_random_state();
        let pkce_verifier = if input.use_pkce {
            debug!("PKCE enabled, generating verifier and challenge");
            Some(generate_pkce_verifier())
        } else {
            None
        };

        let mut auth_url = url::Url::parse(&input.authorization_endpoint).map_err(|e| {
            error!(
                endpoint = %input.authorization_endpoint,
                error = %e,
                "Invalid authorization endpoint URL"
            );
            CredentialError::invalid_input("authorization_endpoint", &e.to_string())
        })?;

        auth_url
            .query_pairs_mut()
            .append_pair("client_id", &input.client_id)
            .append_pair("redirect_uri", &input.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &input.scopes.join(" "))
            .append_pair("state", &state_param);

        if let Some(ref verifier) = pkce_verifier {
            let challenge = generate_code_challenge(verifier);
            auth_url
                .query_pairs_mut()
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
        }

        let mut validation_params = HashMap::new();
        validation_params.insert("state".into(), state_param.clone());

        let partial_state = PartialState {
            data: serde_json::json!({
                "state": state_param,
                "pkce_verifier": pkce_verifier,
                "client_id": input.client_id,
                "client_secret": input.client_secret,
                "token_endpoint": input.token_endpoint,
                "redirect_uri": input.redirect_uri,
            }),
            step: "awaiting_code".into(),
            created_at: unix_now(),
            ttl_seconds: Some(600), // 10 minutes
            metadata: HashMap::new(),
        };

        info!(
            auth_url = %auth_url,
            has_pkce = pkce_verifier.is_some(),
            "Generated authorization URL, awaiting user interaction"
        );

        Ok(InitializeResult::Pending {
            partial_state,
            next_step: InteractionRequest::Redirect {
                url: auth_url.to_string(),
                validation_params,
                metadata: HashMap::new(),
            },
        })
    }

    async fn refresh(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        // Refresh not implemented without storing endpoints
        Err(CredentialError::RefreshNotSupported {
            credential_type: "oauth2_authorization_code".to_string(),
        })
    }
}

/// OAuth2 Authorization Code credential with interactive support
pub struct OAuth2AuthorizationCode {
    flow: AuthorizationCodeFlow,
}

impl OAuth2AuthorizationCode {
    /// Create new OAuth2 Authorization Code credential
    pub fn new() -> Self {
        Self {
            flow: AuthorizationCodeFlow,
        }
    }
}

impl Default for OAuth2AuthorizationCode {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Credential for OAuth2AuthorizationCode {
    type Input = AuthorizationCodeInput;
    type State = OAuth2State;

    fn metadata(&self) -> CredentialMetadata {
        CredentialMetadata {
            key: CredentialKey::new("oauth2_authorization_code")
                .unwrap_or_else(|_| panic!("Invalid credential key")),
            name: "OAuth2 Authorization Code".to_string(),
            description: "OAuth2 authorization code flow with PKCE support".to_string(),
            supports_refresh: true,
            requires_interaction: true,
        }
    }

    async fn initialize(
        &self,
        input: &Self::Input,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        self.flow.execute(input, ctx).await
    }

    async fn refresh(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        // Refresh not supported without storing token endpoint
        Err(CredentialError::RefreshNotSupported {
            credential_type: "oauth2_authorization_code".to_string(),
        })
    }

    async fn revoke(
        &self,
        _state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        // OAuth2 revoke implementation would go here
        Ok(())
    }
}

#[async_trait]
impl InteractiveCredential for OAuth2AuthorizationCode {
    async fn continue_initialization(
        &self,
        partial_state: PartialState,
        user_input: UserInput,
        ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        debug!("Continuing OAuth2 authorization code flow with user input");

        let UserInput::Callback { params } = user_input else {
            error!("Invalid user input type, expected callback");
            return Err(CredentialError::invalid_input(
                "user_input",
                "Expected callback with parameters",
            ));
        };

        // Validate state parameter
        let expected_state: String = serde_json::from_value(partial_state.data["state"].clone())
            .map_err(|e| {
                error!(error = %e, "Failed to deserialize expected state");
                CredentialError::Internal(e.to_string())
            })?;

        let received_state = params.get("state").ok_or_else(|| {
            error!("Missing state parameter in callback");
            CredentialError::InvalidInput {
                field: "state".to_string(),
                reason: "Missing state parameter".to_string(),
            }
        })?;

        if received_state != &expected_state {
            error!(
                expected = %expected_state,
                received = %received_state,
                "State mismatch - possible CSRF attack"
            );
            return Err(CredentialError::InvalidInput {
                field: "state".to_string(),
                reason: "State mismatch - security violation".to_string(),
            });
        }

        debug!("State parameter validated successfully");

        // Extract authorization code
        let code = params
            .get("code")
            .ok_or_else(|| CredentialError::invalid_input("code", "Missing code parameter"))?;

        // Extract stored data
        let token_endpoint: String =
            serde_json::from_value(partial_state.data["token_endpoint"].clone())
                .map_err(|e| CredentialError::Internal(e.to_string()))?;

        let client_id: String = serde_json::from_value(partial_state.data["client_id"].clone())
            .map_err(|e| CredentialError::Internal(e.to_string()))?;

        let redirect_uri: String =
            serde_json::from_value(partial_state.data["redirect_uri"].clone())
                .map_err(|e| CredentialError::Internal(e.to_string()))?;

        // Build token exchange request
        let mut form_data = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.clone()),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
        ];

        // Add PKCE verifier if used
        if let Some(pkce_verifier) = partial_state.data.get("pkce_verifier") {
            if !pkce_verifier.is_null() {
                let verifier: String = serde_json::from_value(pkce_verifier.clone())
                    .map_err(|e| CredentialError::Internal(e.to_string()))?;
                form_data.push(("code_verifier", verifier));
            }
        }

        // Add client_secret if provided
        if let Some(client_secret) = partial_state.data.get("client_secret") {
            if let Some(secret) = client_secret.as_str() {
                form_data.push(("client_secret", secret.to_string()));
            }
        }

        // Exchange code for token
        let response = ctx
            .http_client()
            .post(&token_endpoint)
            .form(&form_data)
            .send()
            .await
            .map_err(|e| CredentialError::NetworkFailed(e.to_string()))?;

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            error!(error = %e, "Failed to read token response body");
            CredentialError::NetworkFailed(e.to_string())
        })?;

        if !status.is_success() {
            error!(
                status = %status,
                body = %body,
                "Token exchange failed"
            );
            return Err(CredentialError::AuthenticationFailed {
                reason: format!("HTTP {} - {}", status, body),
            });
        }

        let token: TokenResponse = serde_json::from_str(&body).map_err(|e| {
            error!(
                error = %e,
                body = %body,
                "Failed to parse token response"
            );
            CredentialError::NetworkFailed(format!(
                "Failed to parse token response: {}. Body: {}",
                e, body
            ))
        })?;

        let state = OAuth2State::from_token_response(token);

        info!("OAuth2 authorization code flow completed successfully");

        Ok(InitializeResult::Complete(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_authorization_code_flow_generates_redirect() {
        let flow = AuthorizationCodeFlow;
        let mut ctx = CredentialContext::new();

        let input = AuthorizationCodeInput {
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            authorization_endpoint: "https://auth.example.com/authorize".to_string(),
            token_endpoint: "https://auth.example.com/token".to_string(),
            redirect_uri: "https://app.example.com/callback".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            use_pkce: true,
        };

        let result = flow.execute(&input, &mut ctx).await.unwrap();

        match result {
            InitializeResult::Pending {
                partial_state,
                next_step,
            } => {
                assert_eq!(partial_state.step, "awaiting_code");
                assert!(partial_state.data["state"].is_string());
                assert!(partial_state.data["pkce_verifier"].is_string());

                if let InteractionRequest::Redirect { url, .. } = next_step {
                    assert!(url.contains("client_id=test_client"));
                    assert!(url.contains("response_type=code"));
                    assert!(url.contains("code_challenge"));
                    assert!(url.contains("code_challenge_method=S256"));
                } else {
                    panic!("Expected Redirect interaction");
                }
            }
            _ => panic!("Expected Pending result"),
        }
    }

    #[tokio::test]
    async fn test_authorization_code_metadata() {
        let cred = OAuth2AuthorizationCode::new();
        let metadata = cred.metadata();

        assert_eq!(metadata.key.as_str(), "oauth2_authorization_code");
        assert!(metadata.supports_refresh);
        assert!(metadata.requires_interaction);
    }
}
