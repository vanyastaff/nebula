//! OAuth2 HTTP helpers for token exchange, device code polling, and refresh.
//!
//! Extracted from the v1 `FlowProtocol` implementation. All functions use
//! v2 error types and operate on the v2 OAuth2State.

// TODO(P10/ADR-0031): relocate reqwest HTTP flow to nebula-api (auth URI
// construct + /oauth/callback endpoint + token exchange) and nebula-engine
// (token refresh during resolve). PKCE primitives stay here.

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use serde_json::Value;
use zeroize::Zeroizing;

use super::{
    config::{AuthStyle, OAuth2Config},
    credential::OAuth2State,
};
use crate::{SecretString, error::CredentialError};

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
/// config MUST come from the `AuthCodeBuilder` in `config`, which
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
    let redirect_uri = config
        .redirect_uri
        .as_deref()
        .ok_or_else(|| provider_error("authorization_code config missing redirect_uri".into()))?;
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
///
/// `client_secret` must be borrowed from a zeroizing buffer (typically
/// `Zeroizing<String>` materialised from a `SecretString` at the caller).
/// This function avoids non-zeroized owned plaintext copies: the form body
/// is built from `&str` borrows, and the `Authorization: Basic …` header
/// intermediates (colon-joined plaintext, BASE64 output, full header value)
/// all live in `Zeroizing<String>` buffers that scrub on drop. `reqwest`
/// still maintains its own URL-encoded body buffer and `HeaderValue` copy,
/// neither of which we can zeroize.
pub(crate) async fn exchange_client_credentials(
    config: &OAuth2Config,
    client_id: &str,
    client_secret: &str,
) -> Result<OAuth2State, CredentialError> {
    let client = http_client();

    let scope_joined: Option<String> = (!config.scopes.is_empty()).then(|| config.scopes.join(" "));
    let mut form: Vec<(&str, &str)> = vec![("grant_type", "client_credentials")];
    if let Some(ref s) = scope_joined {
        form.push(("scope", s));
    }

    let mut req = client.post(&config.token_url);

    match config.auth_style {
        AuthStyle::Header => {
            // Wrap every intermediate — colon-joined plaintext, BASE64 output,
            // and the full `Authorization` header value — in `Zeroizing` so
            // each is scrubbed on drop. `.header(&str, &str)` leaves reqwest
            // to copy bytes into its own `HeaderValue` buffer (out of our
            // hands), but our local copies do not linger as plain `String`s.
            let basic_plaintext: Zeroizing<String> =
                Zeroizing::new(format!("{client_id}:{client_secret}"));
            let credentials: Zeroizing<String> =
                Zeroizing::new(BASE64.encode(basic_plaintext.as_bytes()));
            let auth_header: Zeroizing<String> =
                Zeroizing::new(format!("Basic {}", credentials.as_str()));
            req = req
                .header("Authorization", auth_header.as_str())
                .form(&form);
        },
        AuthStyle::PostBody => {
            form.push(("client_id", client_id));
            form.push(("client_secret", client_secret));
            req = req.form(&form);
        },
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
///
/// `code_verifier` and `client_secret` should be borrowed from zeroizing
/// buffers at the caller — this function builds the form body from `&str`
/// references so no plaintext `String` copy is produced on our heap.
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
            let basic_plaintext: Zeroizing<String> =
                Zeroizing::new(format!("{client_id}:{client_secret}"));
            let credentials: Zeroizing<String> =
                Zeroizing::new(BASE64.encode(basic_plaintext.as_bytes()));
            let auth_header: Zeroizing<String> =
                Zeroizing::new(format!("Basic {}", credentials.as_str()));
            req = req
                .header("Authorization", auth_header.as_str())
                .form(&form);
        },
        AuthStyle::PostBody => {
            req = req.form(&form);
        },
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
///
/// Returns borrowed `&str` slices rather than owned `String`s so the
/// caller's zeroizing buffers are not duplicated onto an extra heap copy
/// (GitHub issue #265).
pub(crate) fn compose_auth_code_form<'a>(
    code: &'a str,
    code_verifier: &'a str,
    redirect_uri: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    auth_style: AuthStyle,
) -> Vec<(&'static str, &'a str)> {
    let mut form: Vec<(&'static str, &'a str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", code_verifier),
        ("redirect_uri", redirect_uri),
    ];
    if matches!(auth_style, AuthStyle::PostBody) {
        form.push(("client_id", client_id));
        form.push(("client_secret", client_secret));
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

    let scope_joined: Option<String> = (!config.scopes.is_empty()).then(|| config.scopes.join(" "));
    let mut form: Vec<(&str, &str)> = vec![("client_id", client_id)];
    if let Some(ref s) = scope_joined {
        form.push(("scope", s));
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

    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", device_code),
    ];

    let mut req = client.post(&config.token_url);

    match config.auth_style {
        AuthStyle::Header => {
            let basic_plaintext: Zeroizing<String> =
                Zeroizing::new(format!("{client_id}:{client_secret}"));
            let credentials: Zeroizing<String> =
                Zeroizing::new(BASE64.encode(basic_plaintext.as_bytes()));
            let auth_header: Zeroizing<String> =
                Zeroizing::new(format!("Basic {}", credentials.as_str()));
            req = req
                .header("Authorization", auth_header.as_str())
                .form(&form);
        },
        AuthStyle::PostBody => {
            form.push(("client_id", client_id));
            form.push(("client_secret", client_secret));
            req = req.form(&form);
        },
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
/// # Secret materialization (GitHub issue #265)
///
/// `refresh_token`, `client_id`, and `client_secret` are materialised into
/// `Zeroizing<String>` buffers that scrub on drop. The form body is built
/// from `&str` borrows into these buffers — we do not give `reqwest`
/// ownership of our plaintext copies. `reqwest` still maintains an internal
/// URL-encoded body buffer (and TLS write buffer) that we cannot zeroize;
/// that residency is bounded by the HTTP round-trip.
///
/// # Errors
///
/// Returns `CredentialError::Provider` if no `refresh_token` is available
/// or if the HTTP request fails.
pub(crate) async fn refresh_token(
    state: &mut OAuth2State,
    config: &OAuth2Config,
) -> Result<(), CredentialError> {
    let refresh_tok: Zeroizing<String> = Zeroizing::new(
        state
            .refresh_token
            .as_ref()
            .ok_or_else(|| provider_error("no refresh_token available for token refresh".into()))?
            .expose_secret(ToOwned::to_owned),
    );
    let client_id: Zeroizing<String> =
        Zeroizing::new(state.client_id.expose_secret(ToOwned::to_owned));
    let client_secret: Zeroizing<String> =
        Zeroizing::new(state.client_secret.expose_secret(ToOwned::to_owned));

    let scope_joined: Option<String> = (!config.scopes.is_empty()).then(|| config.scopes.join(" "));
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_tok.as_str()),
    ];
    if let Some(ref s) = scope_joined {
        form.push(("scope", s));
    }

    let client = http_client();
    let mut req = client.post(&config.token_url);

    match config.auth_style {
        AuthStyle::Header => {
            let basic_plaintext: Zeroizing<String> =
                Zeroizing::new(format!("{}:{}", client_id.as_str(), client_secret.as_str()));
            let credentials: Zeroizing<String> =
                Zeroizing::new(BASE64.encode(basic_plaintext.as_bytes()));
            let auth_header: Zeroizing<String> =
                Zeroizing::new(format!("Basic {}", credentials.as_str()));
            req = req
                .header("Authorization", auth_header.as_str())
                .form(&form);
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
        .map_err(|e| provider_error(format!("refresh token request failed: {e}")))?;

    let body: Value = parse_token_response(resp).await?;
    update_state_from_token_response(state, &body)?;
    Ok(())
}

// ── Internal helpers ────────────────────────────────────────────────────

/// Builds a log-safe summary for a non-2xx OAuth2 token endpoint body.
///
/// Never interpolates the raw response body: some providers echo submitted
/// secrets in error JSON. Only RFC 6749 §5.2 fields are included, with
/// `error_description` truncated (see GitHub issue #277).
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
        // Bound work: at most 257 scalar values, never scan the full provider string.
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

/// Parse an HTTP response as a JSON token response.
///
/// Returns an error if the HTTP status is not 2xx or the body is not valid JSON.
async fn parse_token_response(resp: reqwest::Response) -> Result<Value, CredentialError> {
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        let summary = oauth_token_error_summary(&body_text);
        return Err(provider_error(format!(
            "token endpoint returned {status}: {summary}"
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

    let scopes = body.get("scope").and_then(Value::as_str).map_or_else(
        || default_scopes.to_vec(),
        |s| s.split_whitespace().map(str::to_owned).collect(),
    );

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
/// RFC 6749 §5.1 requires `access_token` in a successful token response; a 2xx
/// body without it is treated as an error so we do not bump `expires_at` while
/// leaving a stale access token (GitHub issue #274).
///
/// Other fields are only overwritten when present. A missing `refresh_token`
/// preserves the existing one (per RFC 6749 Section 6).
fn update_state_from_token_response(
    state: &mut OAuth2State,
    body: &Value,
) -> Result<(), CredentialError> {
    let Some(token) = body.get("access_token").and_then(Value::as_str) else {
        return Err(provider_error(
            "refresh response missing required 'access_token' field".into(),
        ));
    };
    state.access_token = SecretString::new(token);
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
    Ok(())
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
        let challenge = crate::generate_code_challenge(RFC7636_VERIFIER);
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
                ("grant_type", "authorization_code"),
                ("code", "the_code"),
                ("code_verifier", "the_verifier"),
                ("redirect_uri", "https://cb.example/path"),
            ]
        );
    }

    #[test]
    fn compose_auth_code_form_post_body_style_appends_client_credentials() {
        let form = compose_auth_code_form("c", "v", "r", "cid", "csecret", AuthStyle::PostBody);
        assert_eq!(
            form,
            vec![
                ("grant_type", "authorization_code"),
                ("code", "c"),
                ("code_verifier", "v"),
                ("redirect_uri", "r"),
                ("client_id", "cid"),
                ("client_secret", "csecret"),
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

        update_state_from_token_response(&mut state, &body).unwrap();
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

        update_state_from_token_response(&mut state, &body).unwrap();
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
    fn oauth_token_error_summary_omits_raw_body_and_echoed_secrets() {
        let body = r#"{"error":"invalid_client","client_secret":"hunter2"}"#;
        let summary = oauth_token_error_summary(body);
        assert!(
            !summary.contains("hunter2"),
            "secret echoed by provider must not appear: {summary}"
        );
        assert!(summary.contains("invalid_client"), "{summary}");
    }

    #[test]
    fn update_state_errors_when_access_token_missing() {
        let mut state = OAuth2State {
            access_token: SecretString::new("stale"),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("cs"),
            token_url: "https://t.com/token".into(),
            auth_style: AuthStyle::default(),
        };

        let body = serde_json::json!({
            "expires_in": 3600
        });

        let err = update_state_from_token_response(&mut state, &body).unwrap_err();
        assert!(matches!(err, CredentialError::Provider(_)));
        state.access_token.expose_secret(|s| assert_eq!(s, "stale"));
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
