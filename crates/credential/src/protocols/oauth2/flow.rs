//! OAuth2 FlowProtocol implementation.
//!
//! Supports Authorization Code, Client Credentials, and Device Code grant types.

use std::collections::HashMap;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use serde_json::Value;

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;

use crate::core::result::{DisplayData, InitializeResult, InteractionRequest};
use crate::core::{CredentialContext, CredentialError, ValidationError};
use crate::traits::FlowProtocol;

use super::config::{AuthStyle, GrantType, OAuth2Config};
use super::state::OAuth2State;

/// HTTP request timeout for OAuth2 token exchanges.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// OAuth2 [`FlowProtocol`] implementation.
///
/// Supports three grant types:
/// - **Authorization Code** — returns a redirect URL for user browser auth
/// - **Client Credentials** — exchanges credentials for a token directly
/// - **Device Code** — returns a user code + verification URL for CLI/TV apps
pub struct OAuth2Protocol;

impl FlowProtocol for OAuth2Protocol {
    type Config = OAuth2Config;
    type State = OAuth2State;

    fn parameters() -> ParameterCollection {
        let mut client_id = TextParameter::new("client_id", "Client ID");
        client_id.metadata.description = Some("OAuth2 client identifier".into());
        client_id.metadata.required = true;

        let mut client_secret = SecretParameter::new("client_secret", "Client Secret");
        client_secret.metadata.description = Some("OAuth2 client secret".into());
        client_secret.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(client_id))
            .with(ParameterDef::Secret(client_secret))
    }

    async fn initialize(
        config: &Self::Config,
        values: &ParameterValues,
        _ctx: &mut CredentialContext,
    ) -> Result<InitializeResult<Self::State>, CredentialError> {
        let client_id = extract_required(values, "client_id")?;
        let client_secret = extract_required(values, "client_secret")?;

        match config.grant_type {
            GrantType::AuthorizationCode => {
                let url = build_auth_url(config, client_id)?;
                Ok(InitializeResult::RequiresInteraction(
                    InteractionRequest::Redirect {
                        url,
                        validation_params: HashMap::new(),
                        metadata: HashMap::new(),
                    },
                ))
            }
            GrantType::ClientCredentials => {
                let state = exchange_client_credentials(config, client_id, client_secret).await?;
                Ok(InitializeResult::Complete(state))
            }
            GrantType::DeviceCode => {
                let (code, verification_url, expires_in) =
                    request_device_code(config, client_id).await?;
                Ok(InitializeResult::RequiresInteraction(
                    InteractionRequest::DisplayInfo {
                        display_data: DisplayData::UserCode {
                            code: code.clone(),
                            verification_url,
                            complete_url: None,
                        },
                        instructions: Some(format!(
                            "Enter code {code} at the verification URL to authorize this device."
                        )),
                        expires_in,
                    },
                ))
            }
        }
    }

    async fn refresh(
        config: &Self::Config,
        state: &mut Self::State,
        _ctx: &mut CredentialContext,
    ) -> Result<(), CredentialError> {
        let refresh_token =
            state
                .refresh_token
                .as_deref()
                .ok_or_else(|| CredentialError::Validation {
                    source: ValidationError::InvalidFormat(
                        "no refresh_token available for token refresh".into(),
                    ),
                })?;

        let mut form = vec![
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("refresh_token".to_owned(), refresh_token.to_owned()),
        ];

        if !config.scopes.is_empty() {
            form.push(("scope".to_owned(), config.scopes.join(" ")));
        }

        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| http_error(format!("failed to build HTTP client: {e}")))?;

        let resp = client
            .post(&config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| http_error(format!("refresh token request failed: {e}")))?;

        let body: Value = parse_token_response(resp).await?;
        update_state_from_token_response(state, &body);
        Ok(())
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// Extract a required string parameter, returning a validation error if missing.
fn extract_required<'a>(
    values: &'a ParameterValues,
    key: &str,
) -> Result<&'a str, CredentialError> {
    values
        .get_string(key)
        .ok_or_else(|| CredentialError::Validation {
            source: ValidationError::InvalidFormat(format!("missing required field: {key}")),
        })
}

/// Build the authorization URL for the Authorization Code grant.
///
/// Uses [`url::Url`] query encoding so special characters in `client_id`
/// or scope values are always properly percent-encoded.
fn build_auth_url(config: &OAuth2Config, client_id: &str) -> Result<String, CredentialError> {
    let mut url = url::Url::parse(&config.auth_url)
        .map_err(|e| http_error(format!("invalid auth_url: {e}")))?;

    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", client_id);

        if !config.scopes.is_empty() {
            q.append_pair("scope", &config.scopes.join(" "));
        }

        if config.pkce {
            q.append_pair("code_challenge_method", "S256");
        }
    }

    Ok(url.to_string())
}

/// Exchange client credentials for an access token (Client Credentials grant).
async fn exchange_client_credentials(
    config: &OAuth2Config,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuth2State, CredentialError> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| http_error(format!("failed to build HTTP client: {e}")))?;

    let mut form: Vec<(&str, String)> = vec![("grant_type", "client_credentials".into())];

    if !config.scopes.is_empty() {
        form.push(("scope", config.scopes.join(" ")));
    }

    let mut req = client.post(&config.token_url);

    match config.auth_style {
        AuthStyle::Header => {
            let credentials = BASE64.encode(format!("{client_id}:{client_secret}"));
            req = req
                .header("Authorization", format!("Basic {credentials}"))
                .form(&form);
        }
        AuthStyle::PostBody => {
            form.push(("client_id", client_id.to_owned()));
            form.push(("client_secret", client_secret.to_owned()));
            req = req.form(&form);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| http_error(format!("client credentials exchange failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;
    state_from_token_response(&body, &config.scopes)
}

/// Request a device code from the authorization server (Device Code grant).
///
/// Returns `(user_code, verification_url, expires_in)`.
async fn request_device_code(
    config: &OAuth2Config,
    client_id: &str,
) -> Result<(String, String, Option<u64>), CredentialError> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| http_error(format!("failed to build HTTP client: {e}")))?;

    let mut form = vec![("client_id", client_id.to_owned())];
    if !config.scopes.is_empty() {
        form.push(("scope", config.scopes.join(" ")));
    }

    let resp = client
        .post(&config.auth_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| http_error(format!("device code request failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;

    let user_code = body
        .get("user_code")
        .and_then(Value::as_str)
        .ok_or_else(|| http_error("device code response missing 'user_code'".into()))?
        .to_owned();

    // RFC 8628 uses "verification_uri", but some providers use "verification_url".
    let verification_url = body
        .get("verification_uri")
        .or_else(|| body.get("verification_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| http_error("device code response missing 'verification_uri'".into()))?
        .to_owned();

    let expires_in = body.get("expires_in").and_then(Value::as_u64);

    Ok((user_code, verification_url, expires_in))
}

/// Parse an HTTP response as a JSON token response.
///
/// Returns an error if the HTTP status is not 2xx or the body is not valid JSON.
async fn parse_token_response(resp: reqwest::Response) -> Result<Value, CredentialError> {
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(http_error(format!(
            "token endpoint returned {status}: {body_text}"
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| http_error(format!("failed to parse token response: {e}")))
}

/// Build an [`OAuth2State`] from a token endpoint JSON response.
///
/// Falls back to `default_scopes` when the response does not include `scope`.
fn state_from_token_response(
    body: &Value,
    default_scopes: &[String],
) -> Result<OAuth2State, CredentialError> {
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| http_error("token response missing 'access_token'".into()))?
        .to_owned();

    let token_type = body
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or("Bearer")
        .to_owned();

    let refresh_token = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let expires_at = body
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    let scopes = body
        .get("scope")
        .and_then(Value::as_str)
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_else(|| default_scopes.to_vec());

    Ok(OAuth2State {
        access_token,
        token_type,
        refresh_token,
        expires_at,
        scopes,
    })
}

/// Update an existing [`OAuth2State`] from a refresh token response.
///
/// Only overwrites fields present in the response. A missing `refresh_token`
/// preserves the existing one (per RFC 6749 Section 6).
fn update_state_from_token_response(state: &mut OAuth2State, body: &Value) {
    if let Some(token) = body.get("access_token").and_then(Value::as_str) {
        state.access_token = token.to_owned();
    }
    if let Some(tt) = body.get("token_type").and_then(Value::as_str) {
        state.token_type = tt.to_owned();
    }
    if let Some(rt) = body.get("refresh_token").and_then(Value::as_str) {
        state.refresh_token = Some(rt.to_owned());
    }
    if let Some(secs) = body.get("expires_in").and_then(Value::as_u64) {
        state.expires_at = Some(Utc::now() + chrono::Duration::seconds(secs as i64));
    }
    if let Some(scope) = body.get("scope").and_then(Value::as_str) {
        state.scopes = scope.split_whitespace().map(str::to_owned).collect();
    }
}

/// Build a `CredentialError::Validation` from an HTTP/network-related message.
fn http_error(message: String) -> CredentialError {
    CredentialError::Validation {
        source: ValidationError::InvalidFormat(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CredentialContext;
    use crate::protocols::oauth2::config::OAuth2Config;

    #[test]
    fn parameters_has_client_id_and_secret() {
        let params = OAuth2Protocol::parameters();
        assert!(params.contains("client_id"));
        assert!(params.contains("client_secret"));
        assert_eq!(params.len(), 2);
    }

    #[tokio::test]
    async fn authorization_code_returns_redirect() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .scopes(["read"])
            .build();

        let mut values = ParameterValues::new();
        values.set("client_id", serde_json::json!("my_client"));
        values.set("client_secret", serde_json::json!("my_secret"));

        let mut ctx = CredentialContext::new("test");
        let result = OAuth2Protocol::initialize(&config, &values, &mut ctx)
            .await
            .unwrap();

        match result {
            InitializeResult::RequiresInteraction(InteractionRequest::Redirect { url, .. }) => {
                assert!(url.contains("example.com/auth"));
                assert!(url.contains("client_id=my_client"));
                assert!(url.contains("scope=read"));
            }
            other => panic!("expected Redirect, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_client_id_returns_error() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let values = ParameterValues::new(); // empty
        let mut ctx = CredentialContext::new("test");
        let result = OAuth2Protocol::initialize(&config, &values, &mut ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn build_auth_url_includes_pkce() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .pkce(true)
            .build();

        let url = build_auth_url(&config, "cid").unwrap();
        assert!(url.contains("code_challenge_method=S256"));
    }

    #[test]
    fn build_auth_url_without_scopes() {
        let config = OAuth2Config::authorization_code()
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid").unwrap();
        assert!(!url.contains("scope="));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cid"));
    }

    #[test]
    fn state_from_token_response_parses_full() {
        let body = serde_json::json!({
            "access_token": "tok_123",
            "token_type": "Bearer",
            "refresh_token": "ref_456",
            "expires_in": 3600,
            "scope": "read write"
        });

        let state = state_from_token_response(&body, &[]).unwrap();
        assert_eq!(state.access_token, "tok_123");
        assert_eq!(state.token_type, "Bearer");
        assert_eq!(state.refresh_token.as_deref(), Some("ref_456"));
        assert!(state.expires_at.is_some());
        assert_eq!(state.scopes, vec!["read", "write"]);
    }

    #[test]
    fn state_from_token_response_uses_default_scopes() {
        let body = serde_json::json!({
            "access_token": "tok_123"
        });

        let defaults = vec!["read".to_owned()];
        let state = state_from_token_response(&body, &defaults).unwrap();
        assert_eq!(state.scopes, vec!["read"]);
        assert_eq!(state.token_type, "Bearer");
    }

    #[test]
    fn update_state_preserves_existing_refresh_token() {
        let mut state = OAuth2State {
            access_token: "old".into(),
            token_type: "Bearer".into(),
            refresh_token: Some("keep_me".into()),
            expires_at: None,
            scopes: vec![],
        };

        let body = serde_json::json!({
            "access_token": "new_tok"
        });

        update_state_from_token_response(&mut state, &body);
        assert_eq!(state.access_token, "new_tok");
        assert_eq!(state.refresh_token.as_deref(), Some("keep_me"));
    }
}
