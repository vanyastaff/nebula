//! OAuth2 HTTP helpers for token exchange, device code polling, and refresh.
//!
//! Extracted from the v1 `FlowProtocol` implementation. All functions use
//! v2 error types and operate on the v2 OAuth2State.

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use nebula_core::SecretString;
use serde_json::Value;

use super::{
    oauth2::OAuth2State,
    oauth2_config::{AuthStyle, OAuth2Config},
};
use crate::error::CredentialError;

/// HTTP request timeout for OAuth2 token exchanges.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Returns a shared `reqwest::Client` with the standard timeout.
///
/// Lazy-initialized via `OnceLock` so the TLS stack is set up once.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("failed to build HTTP client")
    })
}

/// Build the authorization URL for the Authorization Code grant.
///
/// Appends every query parameter required by RFC 6749 §4.1.1 plus the
/// RFC 7636 PKCE extension and the anti-CSRF `state` parameter. The
/// config MUST come from the `AuthCodeBuilder` in `oauth2_config`, which
/// guarantees that `config.pkce` and `config.redirect_uri` are both
/// `Some(_)` — callers cannot hand us a misconfigured [`OAuth2Config`]
/// without a compile error. The runtime `ok_or_else` branches are there
/// only to defend against struct-literal construction and malformed
/// deserialized records.
///
/// Uses [`url::Url`] query encoding so special characters in
/// `client_id`, `redirect_uri`, and scope values are properly
/// percent-encoded.
pub(crate) fn build_auth_url(
    config: &OAuth2Config,
    client_id: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, CredentialError> {
    let redirect_uri = config.redirect_uri.as_deref().ok_or_else(|| {
        provider_error("authorization_code config missing redirect_uri".into())
    })?;
    let pkce_method = config
        .pkce
        .ok_or_else(|| provider_error("authorization_code config missing pkce method".into()))?;

    let mut url = url::Url::parse(&config.auth_url)
        .map_err(|e| provider_error(format!("invalid auth_url: {e}")))?;

    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", client_id);
        q.append_pair("redirect_uri", redirect_uri);

        if !config.scopes.is_empty() {
            q.append_pair("scope", &config.scopes.join(" "));
        }

        q.append_pair("state", state);
        q.append_pair("code_challenge", code_challenge);
        q.append_pair("code_challenge_method", pkce_method.as_str());
    }

    Ok(url.to_string())
}

/// Exchange client credentials for an access token (Client Credentials grant).
pub(crate) async fn exchange_client_credentials(
    config: &OAuth2Config,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuth2State, CredentialError> {
    let client = http_client();

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
        .map_err(|e| provider_error(format!("client credentials exchange failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;
    state_from_token_response(
        &body,
        &config.scopes,
        client_id,
        client_secret,
        &config.token_url,
        config.auth_style,
    )
}

/// Exchange authorization code for access token (Authorization Code grant).
///
/// `code_verifier` must be the same value whose SHA256 was sent as
/// `code_challenge` in [`build_auth_url`]. `redirect_uri` must byte-match
/// the one on the original auth request (RFC 6749 §4.1.3).
pub(crate) async fn exchange_authorization_code(
    config: &OAuth2Config,
    client_id: &str,
    client_secret: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuth2State, CredentialError> {
    let client = http_client();

    let form = compose_auth_code_form(
        code,
        code_verifier,
        redirect_uri,
        client_id,
        client_secret,
        config.auth_style,
    );

    let mut req = client.post(&config.token_url);

    match config.auth_style {
        AuthStyle::Header => {
            let credentials = BASE64.encode(format!("{client_id}:{client_secret}"));
            req = req
                .header("Authorization", format!("Basic {credentials}"))
                .form(&form);
        }
        AuthStyle::PostBody => {
            req = req.form(&form);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| provider_error(format!("authorization code exchange failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;
    state_from_token_response(
        &body,
        &config.scopes,
        client_id,
        client_secret,
        &config.token_url,
        config.auth_style,
    )
}

/// Build the form body for the authorization-code token exchange.
///
/// Extracted as a pure function so tests can assert the exact shape of
/// the request body without standing up a mock HTTP server. The caller
/// is expected to send this via `reqwest::RequestBuilder::form`.
///
/// Per RFC 6749 §4.1.3 + RFC 7636 §4.5 the form always contains
/// `grant_type=authorization_code`, `code`, `code_verifier`, and
/// `redirect_uri`. When [`AuthStyle::PostBody`] is used, `client_id` and
/// `client_secret` are appended; otherwise they are carried in the
/// `Authorization: Basic` header by the caller.
pub(crate) fn compose_auth_code_form(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    client_secret: &str,
    auth_style: AuthStyle,
) -> Vec<(&'static str, String)> {
    let mut form: Vec<(&'static str, String)> = vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.to_owned()),
        ("code_verifier", code_verifier.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
    ];
    if matches!(auth_style, AuthStyle::PostBody) {
        form.push(("client_id", client_id.to_owned()));
        form.push(("client_secret", client_secret.to_owned()));
    }
    form
}

/// Device code response from authorization server (RFC 8628).
pub(crate) struct DeviceCodeResponse {
    /// The device code for polling.
    pub device_code: String,
    /// The user code to display.
    pub user_code: String,
    /// URL where the user enters the code.
    pub verification_url: String,
    /// Seconds until the device code expires.
    pub expires_in: Option<u64>,
    /// Polling interval in seconds.
    pub interval: u64,
}

/// Request a device code from the authorization server (Device Code grant).
pub(crate) async fn request_device_code(
    config: &OAuth2Config,
    client_id: &str,
) -> Result<DeviceCodeResponse, CredentialError> {
    let client = http_client();

    let mut form = vec![("client_id", client_id.to_owned())];
    if !config.scopes.is_empty() {
        form.push(("scope", config.scopes.join(" ")));
    }

    let resp = client
        .post(&config.auth_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| provider_error(format!("device code request failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;

    let user_code = body
        .get("user_code")
        .and_then(Value::as_str)
        .ok_or_else(|| provider_error("device code response missing 'user_code'".into()))?
        .to_owned();

    // RFC 8628 uses "verification_uri", but some providers use "verification_url".
    let verification_url = body
        .get("verification_uri")
        .or_else(|| body.get("verification_url"))
        .and_then(Value::as_str)
        .ok_or_else(|| provider_error("device code response missing 'verification_uri'".into()))?
        .to_owned();

    let expires_in = body.get("expires_in").and_then(Value::as_u64);
    // RFC 8628: default 5 seconds
    let interval = body.get("interval").and_then(Value::as_u64).unwrap_or(5);

    let device_code = body
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or_else(|| provider_error("device code response missing 'device_code'".into()))?
        .to_owned();

    Ok(DeviceCodeResponse {
        device_code,
        user_code,
        verification_url,
        expires_in,
        interval,
    })
}

/// Result of polling the token endpoint for a device code grant.
pub(crate) enum DevicePollStatus {
    /// Token exchange succeeded.
    Ready(OAuth2State),
    /// User has not yet authorized — keep polling.
    Pending,
    /// Server requests a longer interval.
    SlowDown,
    /// Device code has expired — must restart the flow.
    Expired,
}

/// Poll token endpoint for device code grant (RFC 8628).
///
/// Waits `interval_secs` before polling. Returns a [`DevicePollStatus`]
/// indicating whether the token is ready, the user is still authorizing,
/// or the code has expired.
pub(crate) async fn poll_device_code(
    config: &OAuth2Config,
    client_id: &str,
    client_secret: &str,
    device_code: &str,
    interval_secs: u64,
) -> Result<DevicePollStatus, CredentialError> {
    tokio::time::sleep(Duration::from_secs(interval_secs)).await;

    let client = http_client();

    let mut form: Vec<(&str, String)> = vec![
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:device_code".into(),
        ),
        ("device_code", device_code.to_owned()),
    ];

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
        .map_err(|e| provider_error(format!("device code poll failed: {e}")))?;

    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| provider_error(format!("failed to parse device poll response: {e}")))?;

    if status.is_success() {
        state_from_token_response(
            &body,
            &config.scopes,
            client_id,
            client_secret,
            &config.token_url,
            config.auth_style,
        )
        .map(DevicePollStatus::Ready)
    } else {
        let error = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        match error {
            "authorization_pending" => Ok(DevicePollStatus::Pending),
            "slow_down" => Ok(DevicePollStatus::SlowDown),
            "expired_token" => Ok(DevicePollStatus::Expired),
            _ => Err(provider_error(format!("device code poll error: {error}"))),
        }
    }
}

/// Refresh an OAuth2 access token using the stored refresh token.
///
/// Mutates `state` in place: updates `access_token`, `expires_at`, and
/// optionally `refresh_token` and `token_type` from the response.
///
/// # Errors
///
/// Returns `CredentialError::Provider` if no `refresh_token` is available
/// or if the HTTP request fails.
pub(crate) async fn refresh_token(
    state: &mut OAuth2State,
    config: &OAuth2Config,
) -> Result<(), CredentialError> {
    let refresh_tok = state
        .refresh_token
        .as_ref()
        .ok_or_else(|| provider_error("no refresh_token available for token refresh".into()))?
        .expose_secret(|s| s.to_owned());

    let client_id = state.client_id.expose_secret(|s| s.to_owned());
    let client_secret_str = state.client_secret.expose_secret(|s| s.to_owned());

    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh_tok),
    ];

    if !config.scopes.is_empty() {
        form.push(("scope", config.scopes.join(" ")));
    }

    let client = http_client();
    let mut req = client.post(&config.token_url);

    match config.auth_style {
        AuthStyle::Header => {
            let credentials = BASE64.encode(format!("{client_id}:{client_secret_str}"));
            req = req
                .header("Authorization", format!("Basic {credentials}"))
                .form(&form);
        }
        AuthStyle::PostBody => {
            form.push(("client_id", client_id));
            form.push(("client_secret", client_secret_str));
            req = req.form(&form);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| provider_error(format!("refresh token request failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;
    update_state_from_token_response(state, &body);
    Ok(())
}

// ── Internal helpers ────────────────────────────────────────────────────

/// Parse an HTTP response as a JSON token response.
///
/// Returns an error if the HTTP status is not 2xx or the body is not valid JSON.
async fn parse_token_response(resp: reqwest::Response) -> Result<Value, CredentialError> {
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(provider_error(format!(
            "token endpoint returned {status}: {body_text}"
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| provider_error(format!("failed to parse token response: {e}")))
}

/// Build an [`OAuth2State`] from a token endpoint JSON response.
///
/// Falls back to `default_scopes` when the response does not include `scope`.
/// Embeds `client_id`, `client_secret`, `token_url`, and `auth_style` for
/// later refresh.
fn state_from_token_response(
    body: &Value,
    default_scopes: &[String],
    client_id: &str,
    client_secret: &str,
    token_url: &str,
    auth_style: AuthStyle,
) -> Result<OAuth2State, CredentialError> {
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| provider_error("token response missing 'access_token'".into()))?;

    let token_type = body
        .get("token_type")
        .and_then(Value::as_str)
        .unwrap_or("Bearer")
        .to_owned();

    let refresh_token = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(SecretString::new);

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
        access_token: SecretString::new(access_token),
        token_type,
        refresh_token,
        expires_at,
        scopes,
        client_id: SecretString::new(client_id),
        client_secret: SecretString::new(client_secret),
        token_url: token_url.to_owned(),
        auth_style,
    })
}

/// Update an existing [`OAuth2State`] from a refresh token response.
///
/// Only overwrites fields present in the response. A missing `refresh_token`
/// preserves the existing one (per RFC 6749 Section 6).
fn update_state_from_token_response(state: &mut OAuth2State, body: &Value) {
    if let Some(token) = body.get("access_token").and_then(Value::as_str) {
        state.access_token = SecretString::new(token);
    }
    if let Some(tt) = body.get("token_type").and_then(Value::as_str) {
        state.token_type = tt.to_owned();
    }
    if let Some(rt) = body.get("refresh_token").and_then(Value::as_str) {
        state.refresh_token = Some(SecretString::new(rt));
    }
    if let Some(secs) = body.get("expires_in").and_then(Value::as_u64) {
        state.expires_at = Some(Utc::now() + chrono::Duration::seconds(secs as i64));
    }
    if let Some(scope) = body.get("scope").and_then(Value::as_str) {
        state.scopes = scope.split_whitespace().map(str::to_owned).collect();
    }
}

/// Build a `CredentialError::Provider` from an HTTP/network-related message.
fn provider_error(message: String) -> CredentialError {
    CredentialError::Provider(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALLBACK: &str = "https://app.example.com/cb";

    /// RFC 7636 appendix B vector (section 4.2).
    const RFC7636_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const RFC7636_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn build_auth_url_includes_code_challenge_s256() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid", RFC7636_CHALLENGE, "state_abc").unwrap();
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("code_challenge={RFC7636_CHALLENGE}")));
    }

    #[test]
    fn build_auth_url_includes_state_and_redirect_uri() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid", "chal", "state_abc").unwrap();
        assert!(url.contains("state=state_abc"));
        // `CALLBACK` contains `://` which percent-encodes as `%3A%2F%2F`.
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fcb"),
            "redirect_uri not percent-encoded in URL: {url}"
        );
    }

    #[test]
    fn build_auth_url_verifier_hashes_to_challenge() {
        // Guard against a future refactor breaking the PKCE helper chain.
        let challenge = crate::crypto::generate_code_challenge(RFC7636_VERIFIER);
        assert_eq!(challenge, RFC7636_CHALLENGE);
    }

    #[test]
    fn build_auth_url_without_scopes() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .build();

        let url = build_auth_url(&config, "cid", "chal", "st").unwrap();
        assert!(!url.contains("scope="));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=cid"));
    }

    #[test]
    fn build_auth_url_with_scopes() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("https://example.com/auth")
            .token_url("https://example.com/token")
            .scopes(["read", "write"])
            .build();

        let url = build_auth_url(&config, "cid", "chal", "st").unwrap();
        assert!(url.contains("scope=read+write"));
    }

    #[test]
    fn compose_auth_code_form_header_style_has_exact_shape() {
        let form = compose_auth_code_form(
            "the_code",
            "the_verifier",
            "https://cb.example/path",
            "cid",
            "csecret",
            AuthStyle::Header,
        );
        assert_eq!(
            form,
            vec![
                ("grant_type", "authorization_code".into()),
                ("code", "the_code".into()),
                ("code_verifier", "the_verifier".into()),
                ("redirect_uri", "https://cb.example/path".into()),
            ]
        );
    }

    #[test]
    fn compose_auth_code_form_post_body_style_appends_client_credentials() {
        let form = compose_auth_code_form(
            "c",
            "v",
            "r",
            "cid",
            "csecret",
            AuthStyle::PostBody,
        );
        assert_eq!(
            form,
            vec![
                ("grant_type", "authorization_code".into()),
                ("code", "c".into()),
                ("code_verifier", "v".into()),
                ("redirect_uri", "r".into()),
                ("client_id", "cid".into()),
                ("client_secret", "csecret".into()),
            ]
        );
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

        let state = state_from_token_response(
            &body,
            &[],
            "cid",
            "csecret",
            "https://t.com/token",
            AuthStyle::default(),
        )
        .unwrap();
        state
            .access_token
            .expose_secret(|s| assert_eq!(s, "tok_123"));
        assert_eq!(state.token_type, "Bearer");
        state
            .refresh_token
            .as_ref()
            .unwrap()
            .expose_secret(|s| assert_eq!(s, "ref_456"));
        assert!(state.expires_at.is_some());
        assert_eq!(state.scopes, vec!["read", "write"]);
        state.client_id.expose_secret(|s| assert_eq!(s, "cid"));
        state
            .client_secret
            .expose_secret(|s| assert_eq!(s, "csecret"));
        assert_eq!(state.token_url, "https://t.com/token");
        assert_eq!(state.auth_style, AuthStyle::Header);
    }

    #[test]
    fn state_from_token_response_uses_default_scopes() {
        let body = serde_json::json!({
            "access_token": "tok_123"
        });

        let defaults = vec!["read".to_owned()];
        let state = state_from_token_response(
            &body,
            &defaults,
            "cid",
            "csecret",
            "https://t.com/token",
            AuthStyle::default(),
        )
        .unwrap();
        assert_eq!(state.scopes, vec!["read"]);
        assert_eq!(state.token_type, "Bearer");
    }

    #[test]
    fn update_state_preserves_existing_refresh_token() {
        let mut state = OAuth2State {
            access_token: SecretString::new("old"),
            token_type: "Bearer".into(),
            refresh_token: Some(SecretString::new("keep_me")),
            expires_at: None,
            scopes: vec![],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("cs"),
            token_url: "https://t.com/token".into(),
            auth_style: AuthStyle::default(),
        };

        let body = serde_json::json!({
            "access_token": "new_tok"
        });

        update_state_from_token_response(&mut state, &body);
        state
            .access_token
            .expose_secret(|s| assert_eq!(s, "new_tok"));
        state
            .refresh_token
            .as_ref()
            .unwrap()
            .expose_secret(|s| assert_eq!(s, "keep_me"));
    }

    #[test]
    fn update_state_replaces_all_fields() {
        let mut state = OAuth2State {
            access_token: SecretString::new("old"),
            token_type: "Bearer".into(),
            refresh_token: Some(SecretString::new("old_rt")),
            expires_at: None,
            scopes: vec!["read".into()],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("cs"),
            token_url: "https://t.com/token".into(),
            auth_style: AuthStyle::default(),
        };

        let body = serde_json::json!({
            "access_token": "new_tok",
            "token_type": "mac",
            "refresh_token": "new_rt",
            "expires_in": 1800,
            "scope": "write"
        });

        update_state_from_token_response(&mut state, &body);
        state
            .access_token
            .expose_secret(|s| assert_eq!(s, "new_tok"));
        assert_eq!(state.token_type, "mac");
        state
            .refresh_token
            .as_ref()
            .unwrap()
            .expose_secret(|s| assert_eq!(s, "new_rt"));
        assert!(state.expires_at.is_some());
        assert_eq!(state.scopes, vec!["write"]);
    }

    #[test]
    fn invalid_auth_url_returns_error() {
        let config = OAuth2Config::authorization_code(CALLBACK)
            .auth_url("not a url")
            .token_url("https://t.com/token")
            .build();

        let result = build_auth_url(&config, "cid", "chal", "st");
        assert!(result.is_err());
    }
}
