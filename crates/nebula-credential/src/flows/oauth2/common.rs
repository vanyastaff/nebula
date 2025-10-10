//! Common `OAuth2` types and utilities

use serde::{Deserialize, Serialize};

use crate::core::{CredentialContext, CredentialError, CredentialState, SecureString};

/// `OAuth2` credential state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    pub access_token: SecureString,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<SecureString>,
    pub expires_at: u64,
    #[serde(default = "default_token_type")]
    pub token_type: String,
}

impl CredentialState for OAuth2State {
    const VERSION: u16 = 1;
    const KIND: &'static str = "oauth2";
}

impl OAuth2State {
    /// Create `OAuth2State` from `TokenResponse`
    pub fn from_token_response(token: TokenResponse) -> Self {
        use crate::core::unix_now;
        let expires_in = token.expires_in.unwrap_or(3600); // Default 1 hour
        Self {
            access_token: SecureString::new(token.access_token),
            refresh_token: token.refresh_token.map(SecureString::new),
            expires_at: unix_now() + expires_in,
            token_type: token.token_type.unwrap_or_else(|| "Bearer".to_string()),
        }
    }
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

/// `OAuth2` token response from authorization server
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Refresh `OAuth2` token using `refresh_token` grant
pub async fn oauth2_refresh_token(
    state: &mut OAuth2State,
    ctx: &mut CredentialContext,
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<(), CredentialError> {
    let refresh_token = state
        .refresh_token
        .as_ref()
        .ok_or_else(|| CredentialError::refresh_not_supported("oauth2".to_string()))?;

    let mut form_data = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.expose()),
        ("client_id", client_id),
    ];

    if let Some(secret) = client_secret {
        form_data.push(("client_secret", secret));
    }

    let response = ctx
        .http_client()
        .post(token_endpoint)
        .form(&form_data)
        .send()
        .await
        .map_err(|e| CredentialError::NetworkFailed(e.to_string()))?;

    if !response.status().is_success() {
        return Err(CredentialError::AuthenticationFailed {
            reason: format!("HTTP {}", response.status()),
        });
    }

    let token: TokenResponse = response
        .json()
        .await
        .map_err(|e| CredentialError::NetworkFailed(e.to_string()))?;

    // Update state
    state.access_token = SecureString::new(&token.access_token);
    if let Some(new_refresh) = token.refresh_token {
        state.refresh_token = Some(SecureString::new(&new_refresh));
    }
    let expires_in = token.expires_in.unwrap_or(3600);
    state.expires_at = crate::core::unix_now() + expires_in;
    state.token_type = token.token_type.unwrap_or_else(|| "Bearer".to_string());

    Ok(())
}
