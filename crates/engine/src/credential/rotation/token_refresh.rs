//! Engine-side OAuth2 token refresh.
//!
//! This module hosts the reqwest-based refresh client used by runtime execution
//! paths. Keeping it in `nebula-engine` avoids coupling refresh transport logic
//! to the contract crate.

use chrono::Utc;
use nebula_credential::{
    SecretString,
    credentials::{OAuth2State, oauth2::AuthStyle},
};
use reqwest::Response;
use serde_json::Value;
use zeroize::Zeroizing;

/// Refresh-related failures produced by [`refresh_oauth2_state`].
#[derive(Debug, thiserror::Error)]
pub enum TokenRefreshError {
    /// Stored state lacks a refresh token, so re-auth is required.
    #[error("no refresh_token available for token refresh")]
    MissingRefreshToken,
    /// HTTP request failed.
    #[error("refresh token request failed: {0}")]
    Request(String),
    /// Token endpoint returned non-success status.
    #[error("token endpoint returned {status}: {summary}")]
    TokenEndpoint {
        /// HTTP status code string.
        status: String,
        /// Sanitized RFC6749 error summary.
        summary: String,
    },
    /// Token endpoint response could not be parsed as JSON.
    #[error("failed to parse token response: {0}")]
    Parse(String),
    /// Response body was missing required `access_token`.
    #[error("refresh response missing required 'access_token' field")]
    MissingAccessToken,
}

/// Execute OAuth2 refresh-token grant and mutate `state` in place.
pub async fn refresh_oauth2_state(state: &mut OAuth2State) -> Result<(), TokenRefreshError> {
    let refresh_tok: Zeroizing<String> = Zeroizing::new(
        state
            .refresh_token
            .as_ref()
            .ok_or(TokenRefreshError::MissingRefreshToken)?
            .expose_secret(ToOwned::to_owned),
    );
    let client_id: Zeroizing<String> =
        Zeroizing::new(state.client_id.expose_secret(ToOwned::to_owned));
    let client_secret: Zeroizing<String> =
        Zeroizing::new(state.client_secret.expose_secret(ToOwned::to_owned));

    let scope_joined: Option<String> = (!state.scopes.is_empty()).then(|| state.scopes.join(" "));
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_tok.as_str()),
    ];
    if let Some(ref scope) = scope_joined {
        form.push(("scope", scope.as_str()));
    }

    let client = reqwest::Client::new();
    let mut req = client.post(&state.token_url);
    match state.auth_style {
        AuthStyle::Header => {
            req = req.basic_auth(client_id.as_str(), Some(client_secret.as_str()));
            req = req.form(&form);
        },
        AuthStyle::PostBody => {
            form.push(("client_id", client_id.as_str()));
            form.push(("client_secret", client_secret.as_str()));
            req = req.form(&form);
        },
    }

    let resp = req
        .send()
        .await
        .map_err(|e| TokenRefreshError::Request(e.to_string()))?;
    let body = parse_token_response(resp).await?;
    update_state_from_token_response(state, &body)?;
    Ok(())
}

async fn parse_token_response(resp: Response) -> Result<Value, TokenRefreshError> {
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(TokenRefreshError::TokenEndpoint {
            status: status.to_string(),
            summary: oauth_token_error_summary(&body_text),
        });
    }
    resp.json::<Value>()
        .await
        .map_err(|e| TokenRefreshError::Parse(e.to_string()))
}

fn update_state_from_token_response(
    state: &mut OAuth2State,
    body: &Value,
) -> Result<(), TokenRefreshError> {
    let Some(token) = body.get("access_token").and_then(Value::as_str) else {
        return Err(TokenRefreshError::MissingAccessToken);
    };
    state.access_token = SecretString::new(token);

    if let Some(token_type) = body.get("token_type").and_then(Value::as_str) {
        state.token_type = token_type.to_owned();
    }
    if let Some(refresh_token) = body.get("refresh_token").and_then(Value::as_str) {
        state.refresh_token = Some(SecretString::new(refresh_token));
    }
    if let Some(expires_in) = body.get("expires_in").and_then(Value::as_u64) {
        state.expires_at = Some(Utc::now() + chrono::Duration::seconds(expires_in as i64));
    }
    if let Some(scope) = body.get("scope").and_then(Value::as_str) {
        state.scopes = scope.split_whitespace().map(str::to_owned).collect();
    }

    Ok(())
}

fn oauth_token_error_summary(body_text: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body_text) else {
        return "<non-json body>".to_owned();
    };
    let Some(error) = value.get("error").and_then(Value::as_str) else {
        return "<no error code>".to_owned();
    };

    let mut out = error.to_owned();
    if let Some(desc) = value.get("error_description").and_then(Value::as_str) {
        out.push_str(": ");
        let prefix: Vec<char> = desc.chars().take(257).collect();
        out.extend(prefix.iter().take(256).copied());
        if prefix.len() > 256 {
            out.push('…');
        }
    }
    if let Some(uri) = value.get("error_uri").and_then(Value::as_str) {
        out.push_str(" (error_uri=");
        out.push_str(uri);
        out.push(')');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_state() -> OAuth2State {
        OAuth2State {
            access_token: SecretString::new("old-token"),
            token_type: "Bearer".to_owned(),
            refresh_token: Some(SecretString::new("refresh-1")),
            expires_at: None,
            scopes: vec!["read".to_owned()],
            client_id: SecretString::new("client"),
            client_secret: SecretString::new("secret"),
            token_url: "https://example.com/token".to_owned(),
            auth_style: AuthStyle::Header,
        }
    }

    #[test]
    fn update_state_requires_access_token() {
        let mut state = sample_state();
        let body = serde_json::json!({ "token_type": "Bearer" });
        let err = update_state_from_token_response(&mut state, &body).unwrap_err();
        assert!(matches!(err, TokenRefreshError::MissingAccessToken));
    }

    #[test]
    fn update_state_applies_refresh_response_fields() {
        let mut state = sample_state();
        let body = serde_json::json!({
            "access_token": "new-token",
            "token_type": "Bearer",
            "refresh_token": "refresh-2",
            "expires_in": 3600,
            "scope": "read write",
        });
        update_state_from_token_response(&mut state, &body).expect("response should apply");

        state
            .access_token
            .expose_secret(|v| assert_eq!(v, "new-token"));
        assert_eq!(state.token_type, "Bearer");
        assert_eq!(state.scopes, vec!["read".to_owned(), "write".to_owned()]);
        state
            .refresh_token
            .as_ref()
            .expect("refresh token")
            .expose_secret(|v| assert_eq!(v, "refresh-2"));
        assert!(state.expires_at.is_some());
    }
}
