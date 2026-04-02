//! OAuth2 credential -- interactive, refreshable, multi-grant-type.
//!
//! Supports three OAuth2 grant types via the unified Credential trait:
//! - **Authorization Code** -- user browser redirect (interactive)
//! - **Client Credentials** -- server-to-server, resolves in one step
//! - **Device Code** -- CLI/TV apps, polling flow (interactive)
//!
//! State/scheme separation: OAuth2State is stored (contains refresh
//! internals), while OAuth2Token is the consumer-facing auth material produced by `project()`.

use std::time::Duration;
use std::{fmt, fmt::Formatter};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Parameter, ParameterCollection};

use super::oauth2_config::{AuthStyle, GrantType, OAuth2Config};
use crate::context::CredentialContext;
use crate::credential::Credential;
use crate::description::CredentialDescription;
use crate::error::CredentialError;
use crate::pending::PendingState;
use crate::resolve::{DisplayData, InteractionRequest, RefreshOutcome, ResolveResult, UserInput};
use crate::scheme::OAuth2Token;
use crate::state::CredentialState;
use nebula_core::SecretString;

use super::oauth2_flow;

// ── OAuth2State ────────────────────────────────────────────────────────

/// Internal OAuth2 state with refresh internals.
///
/// This is what gets encrypted and stored. Consumer-facing auth is
/// [`OAuth2Token`] (via [`OAuth2Credential::project`]).
///
/// Contains `client_id`, `client_secret`, and `token_url` so that
/// [`OAuth2Credential::refresh`] can exchange a refresh token without
/// requiring the original setup parameters.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    /// Current access token.
    #[serde(with = "nebula_core::serde_secret")]
    pub access_token: SecretString,
    /// Token type (typically `"Bearer"`).
    pub token_type: String,
    /// Refresh token, if granted by the provider.
    #[serde(default, with = "nebula_core::option_serde_secret")]
    pub refresh_token: Option<SecretString>,
    /// When the access token expires, if known.
    pub expires_at: Option<DateTime<Utc>>,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Stored for refresh operations.
    #[serde(with = "nebula_core::serde_secret")]
    pub client_id: SecretString,
    /// Stored for refresh operations (encrypted at rest via `EncryptionLayer`).
    #[serde(with = "nebula_core::serde_secret")]
    pub client_secret: SecretString,
    /// Token endpoint URL for refresh requests.
    pub token_url: String,
    /// How client credentials are sent (preserved from initial token exchange).
    #[serde(default)]
    pub auth_style: AuthStyle,
}

impl fmt::Debug for OAuth2State {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2State")
            .field("access_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("token_url", &self.token_url)
            .field("auth_style", &self.auth_style)
            .finish()
    }
}

impl OAuth2State {
    /// Returns `true` if the access token is expired or expires within `margin`.
    #[must_use]
    pub fn is_expired(&self, margin: Duration) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => {
                let margin = chrono::Duration::from_std(margin).unwrap_or_default();
                Utc::now() + margin >= exp
            }
        }
    }

    /// `Authorization: Bearer <access_token>` header value.
    #[must_use]
    pub fn bearer_header(&self) -> String {
        self.access_token.expose_secret(|t| format!("Bearer {t}"))
    }
}

impl CredentialState for OAuth2State {
    const KIND: &'static str = "oauth2";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

// ── OAuth2Pending ──────────────────────────────────────────────────────

/// Typed pending state for interactive OAuth2 flows.
///
/// Held in encrypted storage between `resolve()` and `continue_resolve()`.
/// Contains the config + credentials needed to complete the token exchange.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuth2Pending {
    /// OAuth2 provider configuration.
    pub config: OAuth2Config,
    /// OAuth2 client identifier.
    pub client_id: String,
    /// OAuth2 client secret (zeroized on drop).
    #[serde(with = "nebula_core::serde_secret")]
    pub client_secret: SecretString,
    /// Grant type for this pending flow.
    pub grant_type: GrantType,
    /// How client credentials are sent.
    #[serde(default)]
    pub auth_style: AuthStyle,
    /// Device code for device code flow polling.
    pub device_code: Option<String>,
    /// Polling interval in seconds for device code flow.
    pub interval: Option<u64>,
}

impl Zeroize for OAuth2Pending {
    fn zeroize(&mut self) {
        self.client_secret = SecretString::new("");
        if let Some(ref mut dc) = self.device_code {
            dc.zeroize();
        }
    }
}

impl PendingState for OAuth2Pending {
    const KIND: &'static str = "oauth2_pending";

    fn expires_in(&self) -> Duration {
        Duration::from_secs(600) // 10 minutes for interactive flows
    }
}

// ── OAuth2Credential ───────────────────────────────────────────────────

/// OAuth2 credential type implementing the v2 [`Credential`] trait.
///
/// Configuration (auth URL, token URL, grant type, scopes) is provided
/// via [`parameters()`](OAuth2Credential::parameters) and extracted from
/// [`ParameterValues`] in [`resolve()`](OAuth2Credential::resolve).
///
/// # Grant types
///
/// - **Authorization Code** -- returns `Pending` with `Redirect`,
///   completed via `continue_resolve` with callback `code`.
/// - **Client Credentials** -- returns `Complete` immediately.
/// - **Device Code** -- returns `Pending` with `DisplayInfo`,
///   completed via polling `continue_resolve` with `UserInput::Poll`.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::credentials::OAuth2Credential;
/// use nebula_credential::Credential;
///
/// assert_eq!(OAuth2Credential::KEY, "oauth2");
/// assert!(OAuth2Credential::INTERACTIVE);
/// assert!(OAuth2Credential::REFRESHABLE);
/// ```
pub struct OAuth2Credential;

impl Credential for OAuth2Credential {
    type Scheme = OAuth2Token;
    type State = OAuth2State;
    type Pending = OAuth2Pending;

    const KEY: &'static str = "oauth2";
    const INTERACTIVE: bool = true;
    const REFRESHABLE: bool = true;

    fn description() -> CredentialDescription {
        CredentialDescription {
            key: Self::KEY.to_owned(),
            name: "OAuth2".to_owned(),
            description: "OAuth2 authentication supporting Authorization Code, Client Credentials, and Device Code grant types.".to_owned(),
            icon: Some("oauth2".to_owned()),
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(
                Parameter::string("client_id")
                    .label("Client ID")
                    .description("OAuth2 client identifier")
                    .required(),
            )
            .add(
                Parameter::string("client_secret")
                    .label("Client Secret")
                    .description("OAuth2 client secret")
                    .required()
                    .secret(),
            )
            .add(
                Parameter::string("auth_url")
                    .label("Authorization URL")
                    .description("OAuth2 authorization endpoint URL")
                    .placeholder("https://provider.example.com/oauth2/authorize"),
            )
            .add(
                Parameter::string("token_url")
                    .label("Token URL")
                    .description("OAuth2 token endpoint URL")
                    .required()
                    .placeholder("https://provider.example.com/oauth2/token"),
            )
            .add(
                Parameter::string("grant_type")
                    .label("Grant Type")
                    .description(
                        "OAuth2 grant type: authorization_code, client_credentials, or device_code",
                    )
                    .default(serde_json::json!("authorization_code")),
            )
            .add(
                Parameter::string("scopes")
                    .label("Scopes")
                    .description("Space-separated list of OAuth2 scopes"),
            )
    }

    fn project(state: &OAuth2State) -> OAuth2Token {
        let mut token =
            OAuth2Token::new(state.access_token.clone()).with_scopes(state.scopes.clone());

        if let Some(at) = state.expires_at {
            token = token.with_expires_at(at);
        }

        token
    }

    async fn resolve(
        values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        let client_id = extract_required(values, "client_id")?;
        let client_secret = extract_required(values, "client_secret")?;
        let token_url = extract_required(values, "token_url")?;

        let grant_type_str = values
            .get_string("grant_type")
            .unwrap_or("authorization_code");
        let grant_type = parse_grant_type(grant_type_str)?;

        let auth_url = values.get_string("auth_url").unwrap_or_default();
        let scopes = parse_scopes(values);

        let config = build_config(grant_type, auth_url, token_url, &scopes);

        match grant_type {
            GrantType::AuthorizationCode => {
                let url = oauth2_flow::build_auth_url(&config, client_id)?;
                let pending = OAuth2Pending {
                    client_id: client_id.to_owned(),
                    client_secret: SecretString::new(client_secret),
                    grant_type: GrantType::AuthorizationCode,
                    auth_style: config.auth_style,
                    device_code: None,
                    interval: None,
                    config,
                };
                Ok(ResolveResult::Pending {
                    state: pending,
                    interaction: InteractionRequest::Redirect { url },
                })
            }
            GrantType::ClientCredentials => {
                let state =
                    oauth2_flow::exchange_client_credentials(&config, client_id, client_secret)
                        .await?;
                Ok(ResolveResult::Complete(state))
            }
            GrantType::DeviceCode => {
                let device_resp = oauth2_flow::request_device_code(&config, client_id).await?;
                let pending = OAuth2Pending {
                    client_id: client_id.to_owned(),
                    client_secret: SecretString::new(client_secret),
                    grant_type: GrantType::DeviceCode,
                    auth_style: config.auth_style,
                    device_code: Some(device_resp.device_code),
                    interval: Some(device_resp.interval),
                    config,
                };
                Ok(ResolveResult::Pending {
                    state: pending,
                    interaction: InteractionRequest::DisplayInfo {
                        title: "Device Authorization".to_owned(),
                        message: format!(
                            "Enter code {} at the verification URL to authorize this device.",
                            device_resp.user_code,
                        ),
                        data: DisplayData::UserCode {
                            code: device_resp.user_code,
                            verification_uri: device_resp.verification_url,
                            verification_uri_complete: None,
                        },
                        expires_in: device_resp.expires_in,
                    },
                })
            }
        }
    }

    async fn continue_resolve(
        pending: &OAuth2Pending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        match pending.grant_type {
            GrantType::AuthorizationCode => {
                let code = match input {
                    UserInput::Callback { params } => {
                        params.get("code").cloned().ok_or_else(|| {
                            CredentialError::InvalidInput("callback missing 'code' param".into())
                        })?
                    }
                    _ => {
                        return Err(CredentialError::InvalidInput(
                            "authorization_code flow expects UserInput::Callback with 'code'"
                                .into(),
                        ));
                    }
                };
                let client_secret = pending.client_secret.expose_secret(|s| s.to_owned());
                let state = oauth2_flow::exchange_authorization_code(
                    &pending.config,
                    &pending.client_id,
                    &client_secret,
                    &code,
                )
                .await?;
                Ok(ResolveResult::Complete(state))
            }
            GrantType::DeviceCode => {
                if !matches!(input, UserInput::Poll) {
                    return Err(CredentialError::InvalidInput(
                        "device_code flow expects UserInput::Poll".into(),
                    ));
                }
                let device_code = pending.device_code.as_deref().ok_or_else(|| {
                    CredentialError::InvalidInput("pending state missing device_code".into())
                })?;
                let interval = pending.interval.unwrap_or(5);
                let client_secret = pending.client_secret.expose_secret(|s| s.to_owned());

                match oauth2_flow::poll_device_code(
                    &pending.config,
                    &pending.client_id,
                    &client_secret,
                    device_code,
                    interval,
                )
                .await?
                {
                    oauth2_flow::DevicePollStatus::Ready(state) => {
                        Ok(ResolveResult::Complete(state))
                    }
                    oauth2_flow::DevicePollStatus::Pending
                    | oauth2_flow::DevicePollStatus::SlowDown => Ok(ResolveResult::Retry {
                        after: Duration::from_secs(interval),
                    }),
                    oauth2_flow::DevicePollStatus::Expired => {
                        Err(CredentialError::Provider("device code expired".into()))
                    }
                }
            }
            GrantType::ClientCredentials => Err(CredentialError::InvalidInput(
                "client_credentials flow does not use continue_resolve".into(),
            )),
        }
    }

    async fn refresh(
        state: &mut OAuth2State,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        if state.refresh_token.is_none() {
            return Ok(RefreshOutcome::ReauthRequired);
        }

        // Reconstruct minimal config for the refresh call, preserving auth style.
        let config = OAuth2Config::client_credentials()
            .token_url(&state.token_url)
            .auth_style(state.auth_style)
            .scopes(state.scopes.clone())
            .build();

        oauth2_flow::refresh_token(state, &config).await?;
        Ok(RefreshOutcome::Refreshed)
    }
}

// ── Private helpers ────────────────────────────────────────────────────

/// Extract a required string parameter, returning an error if missing.
fn extract_required<'a>(
    values: &'a ParameterValues,
    key: &str,
) -> Result<&'a str, CredentialError> {
    values
        .get_string(key)
        .ok_or_else(|| CredentialError::InvalidInput(format!("missing required field: {key}")))
}

/// Parse a grant type string into the [`GrantType`] enum.
fn parse_grant_type(s: &str) -> Result<GrantType, CredentialError> {
    match s {
        "authorization_code" => Ok(GrantType::AuthorizationCode),
        "client_credentials" => Ok(GrantType::ClientCredentials),
        "device_code" => Ok(GrantType::DeviceCode),
        other => Err(CredentialError::InvalidInput(format!(
            "unknown grant_type: {other}"
        ))),
    }
}

/// Parse space-separated scopes from parameter values.
fn parse_scopes(values: &ParameterValues) -> Vec<String> {
    values
        .get_string("scopes")
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Build an [`OAuth2Config`] from extracted parameter values.
fn build_config(
    grant_type: GrantType,
    auth_url: &str,
    token_url: &str,
    scopes: &[String],
) -> OAuth2Config {
    let builder = match grant_type {
        GrantType::AuthorizationCode => OAuth2Config::authorization_code(),
        GrantType::ClientCredentials => OAuth2Config::client_credentials(),
        GrantType::DeviceCode => OAuth2Config::device_code(),
    };
    builder
        .auth_url(auth_url)
        .token_url(token_url)
        .scopes(scopes.iter().cloned())
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_state() -> OAuth2State {
        OAuth2State {
            access_token: SecretString::new("tok_abc"),
            token_type: "Bearer".into(),
            refresh_token: Some(SecretString::new("ref_xyz")),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(3600)),
            scopes: vec!["read".into(), "write".into()],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("csecret"),
            token_url: "https://example.com/token".into(),
            auth_style: AuthStyle::default(),
        }
    }

    #[test]
    fn key_is_oauth2() {
        assert_eq!(OAuth2Credential::KEY, "oauth2");
    }

    #[test]
    fn capabilities_are_correct() {
        assert!(OAuth2Credential::INTERACTIVE);
        assert!(OAuth2Credential::REFRESHABLE);
        assert!(!OAuth2Credential::REVOCABLE);
        assert!(!OAuth2Credential::TESTABLE);
    }

    #[test]
    fn project_extracts_oauth2_token() {
        let state = make_state();
        let token = OAuth2Credential::project(&state);

        let header = token.bearer_header();
        assert!(header.contains("tok_abc"));
        assert_eq!(token.scopes, vec!["read", "write"]);
        assert!(token.expires_at.is_some());
    }

    #[test]
    fn project_excludes_refresh_internals() {
        let state = make_state();
        let token = OAuth2Credential::project(&state);

        // OAuth2Token should not expose refresh_token, client_id, client_secret
        let serialized = serde_json::to_value(&token).unwrap();
        assert!(serialized.get("refresh_token").is_none());
        assert!(serialized.get("client_id").is_none());
        assert!(serialized.get("client_secret").is_none());
    }

    #[test]
    fn description_has_correct_metadata() {
        let desc = OAuth2Credential::description();
        assert_eq!(desc.key, "oauth2");
        assert_eq!(desc.name, "OAuth2");
        assert!(desc.description.contains("OAuth2"));
    }

    #[test]
    fn parameters_has_all_fields() {
        let params = OAuth2Credential::parameters();
        assert!(params.contains("client_id"));
        assert!(params.contains("client_secret"));
        assert!(params.contains("auth_url"));
        assert!(params.contains("token_url"));
        assert!(params.contains("grant_type"));
        assert!(params.contains("scopes"));
        assert_eq!(params.len(), 6);
    }

    #[test]
    fn parse_grant_type_valid() {
        assert_eq!(
            parse_grant_type("authorization_code").unwrap(),
            GrantType::AuthorizationCode
        );
        assert_eq!(
            parse_grant_type("client_credentials").unwrap(),
            GrantType::ClientCredentials
        );
        assert_eq!(
            parse_grant_type("device_code").unwrap(),
            GrantType::DeviceCode
        );
    }

    #[test]
    fn parse_grant_type_invalid() {
        assert!(parse_grant_type("unknown").is_err());
    }

    #[test]
    fn parse_scopes_empty() {
        let values = ParameterValues::new();
        assert!(parse_scopes(&values).is_empty());
    }

    #[test]
    fn parse_scopes_splits_whitespace() {
        let mut values = ParameterValues::new();
        values.set("scopes", serde_json::json!("read write admin"));
        let scopes = parse_scopes(&values);
        assert_eq!(scopes, vec!["read", "write", "admin"]);
    }

    #[tokio::test]
    async fn resolve_rejects_missing_client_id() {
        let values = ParameterValues::new();
        let ctx = CredentialContext::new("test-user");
        let result = OAuth2Credential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_rejects_missing_token_url() {
        let mut values = ParameterValues::new();
        values.set("client_id", serde_json::json!("cid"));
        values.set("client_secret", serde_json::json!("cs"));
        let ctx = CredentialContext::new("test-user");
        let result = OAuth2Credential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_wrong_input_for_auth_code() {
        let pending = OAuth2Pending {
            config: OAuth2Config::authorization_code()
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: None,
            interval: None,
        };

        let ctx = CredentialContext::new("test-user");
        let result = OAuth2Credential::continue_resolve(&pending, &UserInput::Poll, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_callback_without_code() {
        let pending = OAuth2Pending {
            config: OAuth2Config::authorization_code()
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: None,
            interval: None,
        };

        let ctx = CredentialContext::new("test-user");
        let input = UserInput::Callback {
            params: HashMap::new(),
        };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_wrong_input_for_device_code() {
        let pending = OAuth2Pending {
            config: OAuth2Config::device_code()
                .auth_url("https://a.com/device")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            grant_type: GrantType::DeviceCode,
            auth_style: AuthStyle::default(),
            device_code: Some("dcode".into()),
            interval: Some(5),
        };

        let ctx = CredentialContext::new("test-user");
        let input = UserInput::Callback {
            params: HashMap::new(),
        };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn refresh_returns_reauth_when_no_refresh_token() {
        let mut state = OAuth2State {
            access_token: SecretString::new("tok"),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("cs"),
            token_url: "https://t.com/token".into(),
            auth_style: AuthStyle::default(),
        };

        let ctx = CredentialContext::new("test-user");
        let outcome = OAuth2Credential::refresh(&mut state, &ctx).await.unwrap();
        assert_eq!(outcome, RefreshOutcome::ReauthRequired);
    }

    #[test]
    fn state_is_expired_with_margin() {
        let state = OAuth2State {
            access_token: SecretString::new("tok"),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::seconds(30)),
            scopes: vec![],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("cs"),
            token_url: "https://t.com/token".into(),
            auth_style: AuthStyle::default(),
        };
        // Expires in 30s, margin is 60s => expired
        assert!(state.is_expired(Duration::from_secs(60)));
        // Margin is 0 => not expired
        assert!(!state.is_expired(Duration::from_secs(0)));
    }

    #[test]
    fn no_expiry_never_expired() {
        let state = OAuth2State {
            access_token: SecretString::new("tok"),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("cs"),
            token_url: "https://t.com/token".into(),
            auth_style: AuthStyle::default(),
        };
        assert!(!state.is_expired(Duration::from_secs(9999)));
    }

    #[test]
    fn pending_state_zeroizes_secret() {
        let mut pending = OAuth2Pending {
            config: OAuth2Config::authorization_code()
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("super_secret"),
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: Some("dcode_secret".into()),
            interval: None,
        };

        pending.zeroize();
        pending
            .client_secret
            .expose_secret(|s| assert!(s.is_empty()));
        assert_eq!(pending.device_code.as_deref(), Some(""));
    }

    #[test]
    fn pending_state_expires_in_10_minutes() {
        let pending = OAuth2Pending {
            config: OAuth2Config::authorization_code()
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: None,
            interval: None,
        };

        assert_eq!(pending.expires_in(), Duration::from_secs(600));
    }

    #[test]
    fn credential_state_v2_kind_and_version() {
        assert_eq!(OAuth2State::KIND, "oauth2");
        assert_eq!(OAuth2State::VERSION, 1);
    }

    #[test]
    fn pending_state_kind() {
        assert_eq!(OAuth2Pending::KIND, "oauth2_pending");
    }

    #[test]
    fn bearer_header_format() {
        let state = make_state();
        assert_eq!(state.bearer_header(), "Bearer tok_abc");
    }

    #[test]
    fn oauth2_state_debug_redacts_secrets() {
        let state = make_state();
        let debug = format!("{state:?}");
        assert!(!debug.contains("tok_abc"), "access_token leaked in Debug");
        assert!(!debug.contains("ref_xyz"), "refresh_token leaked in Debug");
        assert!(!debug.contains("csecret"), "client_secret leaked in Debug");
        assert!(debug.contains("[REDACTED]"));
        // Non-secret fields should still appear
        assert!(debug.contains("Bearer"));
        assert!(debug.contains("https://example.com/token"));
    }

    #[test]
    fn oauth2_state_serde_round_trip() {
        let state = make_state();
        let json = serde_json::to_string(&state).unwrap();
        let restored: OAuth2State = serde_json::from_str(&json).unwrap();
        restored
            .access_token
            .expose_secret(|s| assert_eq!(s, "tok_abc"));
        restored
            .refresh_token
            .as_ref()
            .unwrap()
            .expose_secret(|s| assert_eq!(s, "ref_xyz"));
        restored.client_id.expose_secret(|s| assert_eq!(s, "cid"));
        restored
            .client_secret
            .expose_secret(|s| assert_eq!(s, "csecret"));
    }
}
