//! Common `OAuth2` types and utilities

use nebula_log::prelude::error;
use serde::{Deserialize, Serialize};

use crate::core::{CredentialContext, CredentialError, CredentialState, SecureString};

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

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| CredentialError::NetworkFailed(e.to_string()))?;

    if !status.is_success() {
        let sanitized_body = sanitize_response_for_logging(&body);
        error!(
            status = %status,
            body = %sanitized_body,
            "Token refresh failed"
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
            "Failed to parse refresh token response"
        );
        CredentialError::NetworkFailed(format!("Failed to parse token response: {e}"))
    })?;

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
