//! OAuth2 credential with explicit authorization-code and client-credentials acquisition.
//!
//! Models the two grant types whose acquisition paths are implemented:
//! - **Authorization Code** -- user browser redirect (interactive)
//! - **Client Credentials** -- server-to-server, resolves in one step
//!
//! State/scheme separation: OAuth2State is stored (contains refresh
//! internals), while OAuth2Token is the consumer-facing auth material produced by `project()`.

use std::{fmt, fmt::Formatter, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::Context as _;
use nebula_schema::Schema;
// The grant/config types stay reachable from this module because the
// credential and its configuration are one API surface for consumers;
// `AuthStyle` is imported privately from the scheme contract layer that
// owns it.
pub use oauth2_config::{
    AuthCodeBuilder, ClientCredentialsBuilder, GrantType, OAuth2Config, PkceMethod,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::oauth2_config;
use crate::{
    CredentialContext, CredentialPolicy, CredentialState, PendingState, RefreshAttempt,
    RefreshReport, RefreshStrategy, RevokeStrategy, SecretString,
    error::{
        CredentialError, ProviderErrorContext, ProviderErrorKind, RefreshDiagnosticCode,
        RefreshErrorKind, RefreshFailureSpec, RetryAdvice, SecretFreeMessage,
    },
    metadata::CredentialMetadataDraft,
    resolve::{InteractionRequest, ResolveResult, StaticResolveResult, UserInput},
    runtime::refresh::token_refresh::{
        CompletedTokenRefresh, PrepareTokenRefreshError, interpret_oauth2_refresh_response,
        prepare_oauth2_refresh,
    },
    runtime::{OAUTH_ENDPOINT_MAX_BYTES, OAuthServerEndpoint, TokenPostRequest, TokenPostResponse},
    scheme::{AuthStyle, OAuth2Token},
};

// ── OAuth2State ────────────────────────────────────────────────────────

/// Internal OAuth2 state with refresh internals.
///
/// This is what gets encrypted and stored. Consumer-facing auth is
/// [`OAuth2Token`] (via [`Credential::project`](crate::Credential::project)).
///
/// Contains `client_id`, `client_secret`, and `token_url` so that
/// [`Refreshable::refresh`](crate::Refreshable::refresh) can exchange a refresh
/// token without requiring the original setup parameters.
///
/// Per Tech Spec §15.4 amendment — `Zeroize` + `ZeroizeOnDrop` derived
/// so the decrypted plaintext (access/refresh tokens, client creds)
/// is scrubbed deterministically when this state is dropped. Non-secret
/// fields (token type, expiry, scopes, auth-style enum) carry
/// `#[zeroize(skip)]`. `token_url` is scrubbed because provider-routing query
/// parameters can contain tenant or credential-adjacent values.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct OAuth2State {
    /// Current access token.
    #[serde(with = "crate::serde_secret")]
    pub access_token: SecretString,
    /// Token type (typically `"Bearer"`) — non-secret marker.
    #[zeroize(skip)]
    pub token_type: String,
    /// Refresh token, if granted by the provider.
    #[serde(default, with = "crate::serde_secret::option")]
    pub refresh_token: Option<SecretString>,
    /// When the access token expires, if known — non-secret timestamp.
    #[zeroize(skip)]
    pub expires_at: Option<DateTime<Utc>>,
    /// Granted scopes — non-secret list of OAuth2 scope identifiers.
    #[zeroize(skip)]
    pub scopes: Vec<String>,
    /// Stored for refresh operations.
    #[serde(with = "crate::serde_secret")]
    pub client_id: SecretString,
    /// Stored for refresh operations (encrypted at rest via `EncryptionLayer`).
    #[serde(with = "crate::serde_secret")]
    pub client_secret: SecretString,
    /// Token endpoint URL for refresh requests. Query parameters are treated
    /// as sensitive routing material and zeroized with the state.
    pub token_url: String,
    /// How client credentials are sent (preserved from initial token
    /// exchange) — non-secret enum discriminant.
    #[zeroize(skip)]
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
            .field("token_url", &"[REDACTED]")
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
            },
        }
    }

    /// `Authorization: Bearer <access_token>` header value.
    ///
    /// Per Tech Spec §15.5 (closes security-lead N4): the bearer header
    /// contains the access token verbatim; returning `SecretString` forces
    /// `.expose_secret()` at the FFI boundary, eliminating accidental
    /// `Debug` / log leaks of the bearer string. Symmetric with
    /// [`OAuth2Token::bearer_header`](crate::scheme::OAuth2Token::bearer_header).
    ///
    /// SEC-09 (security hardening 2026-04-27 Stage 2): construction goes
    /// through a `Zeroizing<String>` buffer instead of `format!` so that any
    /// panic during string assembly zeros the partial bearer; the only
    /// non-zeroizing window remaining is the single-instruction move into
    /// `SecretString::new`, which has no yield/alloc point inside it.
    #[must_use]
    pub fn bearer_header(&self) -> SecretString {
        let token = self.access_token.expose_secret();
        let mut buf = zeroize::Zeroizing::new(String::with_capacity(7 + token.len()));
        buf.push_str("Bearer ");
        buf.push_str(token);
        SecretString::new(std::mem::take(&mut *buf))
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
///
/// The type represents only authorization-code continuation, so PKCE, CSRF
/// state, and redirect URI are required fields rather than optional sentinels.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuth2Pending {
    /// OAuth2 provider configuration.
    pub config: OAuth2Config,
    /// OAuth2 client identifier.
    pub client_id: String,
    /// OAuth2 client secret (zeroized on drop).
    #[serde(with = "crate::serde_secret")]
    pub client_secret: SecretString,
    /// How client credentials are sent.
    pub auth_style: AuthStyle,
    /// PKCE code verifier for AuthorizationCode flows.
    ///
    /// Generated fresh on every `resolve()`. Sent as `code_verifier` on
    /// the token exchange so the provider can recompute and match the
    /// `code_challenge` carried on the auth URL.
    #[serde(with = "crate::serde_secret")]
    pub pkce_verifier: SecretString,
    /// Anti-CSRF `state` parameter for AuthorizationCode flows.
    ///
    /// Generated fresh on every `resolve()`. Validated in
    /// `continue_resolve` against the callback-provided value via a
    /// constant-time comparison.
    pub state: String,
    /// Exact `redirect_uri` echoed on the token exchange for
    /// AuthorizationCode flows.
    pub redirect_uri: String,
}

// Constant `Debug` keeps URLs (including provider-routing query parameters),
// client material, device bearer material, PKCE, state, and redirect values
// out of diagnostics without leaking their presence or length.
impl fmt::Debug for OAuth2Pending {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("OAuth2Pending(<redacted>)")
    }
}

impl Zeroize for OAuth2Pending {
    fn zeroize(&mut self) {
        self.config.zeroize();
        // Zeroize the existing SecretString in place before replacing it, so
        // the underlying heap buffer is scrubbed rather than relying solely
        // on Drop of the replacement.
        self.client_secret.zeroize();
        self.client_secret = SecretString::new("");
        // client_id is not strictly a secret, but it correlates to an
        // account and we are told to wipe this struct — scrub it too.
        self.client_id.zeroize();
        self.pkce_verifier.zeroize();
        self.pkce_verifier = SecretString::new("");
        self.state.zeroize();
        self.redirect_uri.zeroize();
    }
}

// Per Tech Spec §15.4 — `PendingState: ZeroizeOnDrop`. Hand-rolled
// (rather than `#[derive(ZeroizeOnDrop)]`) because the derive emits a
// field-by-field `Drop` body and would not preserve the mixed-secret /
// non-secret zeroize logic in the manual `Zeroize` impl above (drop
// `Option`s to `None`, swap `client_secret` for an empty
// `SecretString` so the heap buffer is scrubbed in place, etc.).
impl Drop for OAuth2Pending {
    fn drop(&mut self) {
        self.zeroize();
    }
}
impl ZeroizeOnDrop for OAuth2Pending {}

impl PendingState for OAuth2Pending {
    const KIND: &'static str = "oauth2_pending";

    fn expires_in(&self) -> Duration {
        Duration::from_mins(10) // 10 minutes for interactive flows
    }
}

// ── OAuth2Credential ───────────────────────────────────────────────────

/// OAuth2 credential type implementing the [`Credential`](crate::Credential)
/// trait plus the [`Interactive`](crate::Interactive) and
/// [`Refreshable`](crate::Refreshable) sub-traits. Initial exchange and refresh
/// have separate injected transport capabilities because only refresh owns the
/// provider/persistence critical-section authority.
///
/// # Grant types and entry points
///
/// - **Authorization Code** enters through [`Interactive::begin`](crate::Interactive::begin),
///   persists [`OAuth2Pending`], then exchanges the callback code in
///   [`Interactive::continue_resolve`](crate::Interactive::continue_resolve).
/// - **Client Credentials** performs one token exchange in
///   [`Credential::resolve`](crate::Credential::resolve).
pub struct OAuth2Credential;

/// Typed shape of the `oauth2` credential setup form.
#[derive(Schema, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuth2Properties {
    /// Browser authorization-code flow with mandatory PKCE and redirect URI.
    AuthorizationCode(OAuth2AuthorizationCodeProperties),
    /// Server-to-server client-credentials exchange.
    ClientCredentials(OAuth2ClientCredentialsProperties),
}

/// Reusable OAuth2 client identity and secret pair.
#[derive(Schema, Deserialize)]
pub struct OAuth2ClientProperties {
    /// OAuth2 client identifier.
    #[field(label = "Client ID")]
    #[validate(required, length(max = 4096))]
    pub client_id: String,
    /// OAuth2 client secret retained in zeroizing memory.
    #[field(secret, label = "Client Secret")]
    #[validate(required)]
    pub client_secret: SecretString,
}

/// Typed authorization-code setup properties.
#[derive(Schema, Deserialize)]
pub struct OAuth2AuthorizationCodeProperties {
    /// OAuth2 client identity and secret.
    pub client: OAuth2ClientProperties,
    /// Authorization endpoint URL.
    #[validate(required, url, length(max = 8192))]
    pub auth_url: String,
    /// Token endpoint URL.
    #[validate(required, url, length(max = 8192))]
    pub token_url: String,
    /// Requested scopes.
    pub scopes: Option<Vec<String>>,
    /// Registered callback URI.
    #[validate(required, url, length(max = 8192))]
    pub redirect_uri: String,
    /// Explicit client-authentication placement.
    #[field(enum_select)]
    pub auth_style: AuthStyle,
}

/// Typed client-credentials setup properties.
#[derive(Schema, Deserialize)]
pub struct OAuth2ClientCredentialsProperties {
    /// OAuth2 client identity and secret.
    pub client: OAuth2ClientProperties,
    /// Token endpoint URL.
    #[validate(required, url, length(max = 8192))]
    pub token_url: String,
    /// Requested scopes.
    pub scopes: Option<Vec<String>>,
    /// Explicit client-authentication placement.
    #[field(enum_select)]
    pub auth_style: AuthStyle,
}

// ADR-0088 D1: the full OAuth2 credential surface in one `impl` block.
// `#[credential]` sees `begin` + `continue_resolve` (+ `type Pending`) and
// `refresh`, and emits only the implemented `Interactive` + `Refreshable`
// capability impls.
// The hand-written `policy()` is relocated verbatim because OAuth2's refresh
// strategy is state-dependent (`RefreshToken` while a refresh token is held,
// else `ReAcquire`) — the macro's synthesized policy cannot read live state.
// The `initiate_authorization_code` building block stays in its own inherent
// `impl` block below. It is not part of the credential contract and does not
// imply a public provider-specific HTTP kickoff surface.
#[nebula_credential::credential(key = "oauth2")]
impl OAuth2Credential {
    type Properties = OAuth2Properties;
    type Scheme = OAuth2Token;
    type State = OAuth2State;

    fn metadata() -> CredentialMetadataDraft {
        CredentialMetadataDraft::new(
            nebula_core::credential_key!("oauth2"),
            crate::metadata_name!("OAuth2"),
            "OAuth2 authentication supporting Authorization Code and Client Credentials grant types.",
        )
        .with_icon(nebula_metadata::Icon::inline("oauth2"))
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
        properties: &OAuth2Properties,
        ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<OAuth2State>, CredentialError> {
        let OAuth2Properties::ClientCredentials(properties) = properties else {
            return Err(CredentialError::InteractiveRequired);
        };
        acquire_client_credentials(properties, ctx)
            .await
            .map(StaticResolveResult::Complete)
    }

    type Pending = OAuth2Pending;

    async fn begin(
        properties: &OAuth2Properties,
        ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        match properties {
            OAuth2Properties::AuthorizationCode(properties) => {
                let (state, interaction) = initiate_authorization_code(properties)?;
                Ok(ResolveResult::Pending { state, interaction })
            },
            OAuth2Properties::ClientCredentials(properties) => {
                acquire_client_credentials(properties, ctx)
                    .await
                    .map(ResolveResult::Complete)
            },
        }
    }

    async fn continue_resolve(
        pending: &OAuth2Pending,
        input: &UserInput,
        ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        let params = match input {
            UserInput::Callback { params } => params,
            _ => return Err(CredentialError::InvalidInput),
        };
        const MAX_CALLBACK_PARAMETERS: usize = 16;
        const MAX_CALLBACK_BYTES: usize = 64 * 1024;
        const MAX_AUTHORIZATION_CODE_BYTES: usize = 16 * 1024;
        let callback_bytes = params.iter().try_fold(0_usize, |total, (key, value)| {
            total.checked_add(key.len())?.checked_add(value.len())
        });
        if params.len() > MAX_CALLBACK_PARAMETERS
            || callback_bytes.is_none_or(|bytes| bytes > MAX_CALLBACK_BYTES)
        {
            return Err(CredentialError::InvalidInput);
        }
        let code = params.get("code").ok_or(CredentialError::InvalidInput)?;
        let callback_state = params.get("state").ok_or(CredentialError::InvalidInput)?;
        if code.is_empty()
            || code.len() > MAX_AUTHORIZATION_CODE_BYTES
            || !code.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
            || pending.state.len() != 43
            || callback_state.len() != pending.state.len()
        {
            return Err(CredentialError::InvalidInput);
        }
        let state_matches: bool = callback_state
            .as_bytes()
            .ct_eq(pending.state.as_bytes())
            .into();
        if !state_matches {
            return Err(CredentialError::InvalidInput);
        }

        let form = vec![
            (
                "grant_type".to_owned(),
                SecretString::new("authorization_code"),
            ),
            ("code".to_owned(), SecretString::new(code)),
            (
                "redirect_uri".to_owned(),
                SecretString::new(&pending.redirect_uri),
            ),
            ("code_verifier".to_owned(), pending.pkce_verifier.clone()),
        ];
        let request = build_token_request(
            &pending.config.token_url,
            &pending.client_id,
            &pending.client_secret,
            pending.auth_style,
            form,
        )?;
        let state = dispatch_acquisition(
            request,
            &pending.client_id,
            &pending.client_secret,
            &pending.config.token_url,
            pending.auth_style,
            &pending.config.scopes,
            ctx,
        )
        .await?;
        Ok(ResolveResult::Complete(state))
    }

    async fn refresh(state: &mut OAuth2State, attempt: RefreshAttempt<'_>) -> RefreshReport {
        let prepared = match prepare_oauth2_refresh(state) {
            Ok(prepared) => prepared,
            Err(PrepareTokenRefreshError::MissingRefreshToken) => {
                return attempt.missing_refresh_material();
            },
            Err(PrepareTokenRefreshError::InvalidRefreshToken) => {
                return attempt.not_dispatched(refresh_failure_spec(
                    RefreshErrorKind::ProtocolError,
                    RetryAdvice::Never,
                    "oauth.invalid_refresh_token",
                ));
            },
            Err(PrepareTokenRefreshError::InvalidScopes) => {
                return attempt.not_dispatched(refresh_failure_spec(
                    RefreshErrorKind::ProtocolError,
                    RetryAdvice::Never,
                    "oauth.invalid_scopes",
                ));
            },
            Err(PrepareTokenRefreshError::InvalidEndpoint(_)) => {
                return attempt.not_dispatched(refresh_failure_spec(
                    RefreshErrorKind::ProtocolError,
                    RetryAdvice::Never,
                    "oauth.invalid_endpoint",
                ));
            },
        };

        let Some(transport) = attempt.context().refresh_transport() else {
            let retry = crate::RetryDelay::new(Duration::from_mins(1))
                .map(RetryAdvice::After)
                .unwrap_or(RetryAdvice::Never);
            return attempt.not_dispatched(refresh_failure_spec(
                RefreshErrorKind::ProviderUnavailable,
                retry,
                "oauth.transport_not_configured",
            ));
        };

        let completed = match attempt
            .dispatch(|| transport.post_token(prepared.into_request()))
            .await
        {
            Ok(completed) => completed,
            Err(unknown) => return unknown.into_report(),
        };
        let (response, proof) = completed.into_parts();
        match interpret_oauth2_refresh_response(state, response) {
            CompletedTokenRefresh::Refreshed => proof.refreshed(),
            CompletedTokenRefresh::InvalidGrant { .. } => proof.provider_rejected(),
            CompletedTokenRefresh::DefinitiveNoEffect { code, .. } => {
                proof.confirmed_not_applied(refresh_failure_spec(
                    RefreshErrorKind::ProtocolError,
                    RetryAdvice::Never,
                    code.as_str(),
                ))
            },
            CompletedTokenRefresh::AmbiguousDenial { .. }
            | CompletedTokenRefresh::MalformedSuccess { .. } => proof.outcome_unknown(),
        }
    }

    // OAuth2 is a refresh-pair credential (ADR-0088 D2). The policy is computed
    // from live state: `RefreshToken` while a refresh token is held (the runtime
    // can renew non-interactively), otherwise `ReAcquire` (the refresh path
    // returns `ReauthRequired`). Provider revocation is not implemented; expiry
    // is the access token's inline `expires_at`. The hand-written `policy` is kept
    // (not macro-synthesized) precisely because the refresh strategy depends on
    // live state, which the macro's synthesized default cannot read.
    fn policy(state: &OAuth2State) -> CredentialPolicy {
        CredentialPolicy {
            expires_at: state.expires_at,
            lease: None,
            refresh: if state.refresh_token.is_some() {
                RefreshStrategy::RefreshToken
            } else {
                // No refresh token: re-acquisition is a human-gated OAuth2
                // re-authorization, not a federated exchange of another credential.
                RefreshStrategy::ReAcquire {
                    from: None,
                    interactive: true,
                }
            },
            revoke: RevokeStrategy::None,
        }
    }
}

// ── Private helpers ────────────────────────────────────────────────────

fn initiate_authorization_code(
    properties: &OAuth2AuthorizationCodeProperties,
) -> Result<(OAuth2Pending, InteractionRequest), CredentialError> {
    validate_client(&properties.client)?;
    let scopes = properties.scopes.as_deref().unwrap_or_default();
    validate_requested_scopes(scopes)?;
    let config = OAuth2Config::authorization_code(properties.redirect_uri.clone())
        .auth_url(&properties.auth_url)
        .token_url(&properties.token_url)
        .scopes(scopes.iter().cloned())
        .auth_style(properties.auth_style)
        .build();
    let verifier = crate::generate_pkce_verifier();
    let challenge = crate::generate_code_challenge(&verifier);
    let state_token = crate::generate_random_state();
    let url = build_auth_url(
        &config,
        &properties.client.client_id,
        &challenge,
        &state_token,
    )?;
    let pending = OAuth2Pending {
        config,
        client_id: properties.client.client_id.clone(),
        client_secret: properties.client.client_secret.clone(),
        auth_style: properties.auth_style,
        pkce_verifier: SecretString::new(verifier),
        state: state_token,
        redirect_uri: properties.redirect_uri.clone(),
    };
    Ok((pending, InteractionRequest::Redirect { url }))
}

async fn acquire_client_credentials(
    properties: &OAuth2ClientCredentialsProperties,
    ctx: &CredentialContext,
) -> Result<OAuth2State, CredentialError> {
    validate_client(&properties.client)?;
    let scopes = properties.scopes.as_deref().unwrap_or_default();
    validate_requested_scopes(scopes)?;
    let mut form = vec![(
        "grant_type".to_owned(),
        SecretString::new("client_credentials"),
    )];
    if !scopes.is_empty() {
        form.push(("scope".to_owned(), SecretString::new(scopes.join(" "))));
    }
    let request = build_token_request(
        &properties.token_url,
        &properties.client.client_id,
        &properties.client.client_secret,
        properties.auth_style,
        form,
    )?;
    dispatch_acquisition(
        request,
        &properties.client.client_id,
        &properties.client.client_secret,
        &properties.token_url,
        properties.auth_style,
        scopes,
        ctx,
    )
    .await
}

fn build_token_request(
    token_url: &str,
    client_id: &str,
    client_secret: &SecretString,
    auth_style: AuthStyle,
    mut form: Vec<(String, SecretString)>,
) -> Result<TokenPostRequest, CredentialError> {
    let endpoint = OAuthServerEndpoint::parse(token_url).map_err(|_| {
        CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::Schema,
            SecretFreeMessage::new("invalid OAuth2 token endpoint URL"),
        )))
    })?;
    let basic_auth = match auth_style {
        AuthStyle::Header => Some((
            encode_basic_component(client_id),
            encode_basic_component(client_secret.expose_secret()),
        )),
        AuthStyle::PostBody => {
            form.push(("client_id".to_owned(), SecretString::new(client_id)));
            form.push(("client_secret".to_owned(), client_secret.clone()));
            None
        },
    };
    Ok(TokenPostRequest::new(endpoint, form, basic_auth))
}

fn encode_basic_component(raw: &str) -> SecretString {
    let mut encoded = zeroize::Zeroizing::new(String::with_capacity(raw.len()));
    for part in url::form_urlencoded::byte_serialize(raw.as_bytes()) {
        encoded.push_str(part);
    }
    SecretString::new(std::mem::take(&mut *encoded))
}

async fn dispatch_acquisition(
    request: TokenPostRequest,
    client_id: &str,
    client_secret: &SecretString,
    token_url: &str,
    auth_style: AuthStyle,
    requested_scopes: &[String],
    ctx: &CredentialContext,
) -> Result<OAuth2State, CredentialError> {
    let Some(transport) = ctx.acquisition_transport() else {
        return Err(CredentialError::AcquisitionTransportUnavailable);
    };
    tracing::debug!("dispatching OAuth2 credential acquisition");
    let response = transport
        .post_token(request)
        .await
        .map_err(|_| CredentialError::OutcomeUnknown)?;
    interpret_acquisition_response(
        response,
        client_id,
        client_secret,
        token_url,
        auth_style,
        requested_scopes,
        ctx.clock().now(),
    )
}

#[derive(Deserialize, Zeroize)]
struct AcquisitionTokenResponse {
    access_token: Option<SecretString>,
    token_type: Option<SecretString>,
    refresh_token: Option<SecretString>,
    expires_in: Option<u64>,
    scope: Option<SecretString>,
}

impl Drop for AcquisitionTokenResponse {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl fmt::Debug for AcquisitionTokenResponse {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("AcquisitionTokenResponse(<redacted>)")
    }
}

fn interpret_acquisition_response(
    response: TokenPostResponse,
    client_id: &str,
    client_secret: &SecretString,
    token_url: &str,
    auth_style: AuthStyle,
    requested_scopes: &[String],
    now: DateTime<Utc>,
) -> Result<OAuth2State, CredentialError> {
    if !(200..300).contains(&response.status()) {
        let kind = if response.status() == 400 || response.status() == 401 {
            ProviderErrorKind::Auth
        } else {
            ProviderErrorKind::ServerError
        };
        return Err(CredentialError::Provider(Box::new(
            ProviderErrorContext::new(
                kind,
                SecretFreeMessage::new("OAuth2 token acquisition was rejected by the provider"),
            ),
        )));
    }

    let mut body: AcquisitionTokenResponse = serde_json::from_slice(response.body().as_ref())
        .map_err(|_| CredentialError::OutcomeUnknown)?;
    let access_token = body
        .access_token
        .take()
        .filter(|token| is_visible_token(token.expose_secret()))
        .ok_or(CredentialError::OutcomeUnknown)?;
    let token_type = body
        .token_type
        .as_ref()
        .filter(|token_type| token_type.expose_secret().eq_ignore_ascii_case("bearer"))
        .ok_or(CredentialError::OutcomeUnknown)?;
    let _ = token_type;
    if body
        .refresh_token
        .as_ref()
        .is_some_and(|token| !is_visible_token(token.expose_secret()))
    {
        return Err(CredentialError::OutcomeUnknown);
    }
    let expires_at = body
        .expires_in
        .map(|expires_in| {
            let seconds = i64::try_from(expires_in).map_err(|_| CredentialError::OutcomeUnknown)?;
            now.checked_add_signed(chrono::Duration::seconds(seconds))
                .ok_or(CredentialError::OutcomeUnknown)
        })
        .transpose()?;
    let scopes = parse_granted_scopes(body.scope.as_ref(), requested_scopes)?;

    Ok(OAuth2State {
        access_token,
        token_type: "Bearer".to_owned(),
        refresh_token: body.refresh_token.take(),
        expires_at,
        scopes,
        client_id: SecretString::new(client_id),
        client_secret: client_secret.clone(),
        token_url: token_url.to_owned(),
        auth_style,
    })
}

fn parse_granted_scopes(
    returned: Option<&SecretString>,
    requested: &[String],
) -> Result<Vec<String>, CredentialError> {
    let Some(returned) = returned else {
        return Ok(requested.to_vec());
    };
    let raw = returned.expose_secret();
    if raw.is_empty() {
        return Err(CredentialError::OutcomeUnknown);
    }
    const MAX_SCOPE_COUNT: usize = 64;
    const MAX_SCOPE_BYTES: usize = 256;
    const MAX_TOTAL_SCOPE_BYTES: usize = 4 * 1024;
    if raw.len() > MAX_TOTAL_SCOPE_BYTES {
        return Err(CredentialError::OutcomeUnknown);
    }
    let mut granted = Vec::new();
    for scope in raw.split(' ') {
        let valid = !scope.is_empty()
            && scope.len() <= MAX_SCOPE_BYTES
            && scope
                .bytes()
                .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e));
        if !valid
            || (!requested.is_empty() && !requested.iter().any(|candidate| candidate == scope))
            || granted.iter().any(|seen| seen == scope)
            || granted.len() >= MAX_SCOPE_COUNT
        {
            return Err(CredentialError::OutcomeUnknown);
        }
        granted.push(scope.to_owned());
    }
    Ok(granted)
}

fn validate_requested_scopes(scopes: &[String]) -> Result<(), CredentialError> {
    const MAX_SCOPE_COUNT: usize = 64;
    const MAX_SCOPE_BYTES: usize = 256;
    const MAX_TOTAL_SCOPE_BYTES: usize = 4 * 1024;
    let total = scopes.iter().try_fold(0_usize, |total, scope| {
        total.checked_add(scope.len())?.checked_add(1)
    });
    let valid = scopes.len() <= MAX_SCOPE_COUNT
        && total.is_some_and(|total| total <= MAX_TOTAL_SCOPE_BYTES)
        && scopes.iter().enumerate().all(|(index, scope)| {
            !scope.is_empty()
                && scope.len() <= MAX_SCOPE_BYTES
                && scope
                    .bytes()
                    .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
                && !scopes[..index].contains(scope)
        });
    if valid {
        Ok(())
    } else {
        Err(CredentialError::Provider(Box::new(
            ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("invalid OAuth2 requested scopes"),
            ),
        )))
    }
}

fn validate_client(client: &OAuth2ClientProperties) -> Result<(), CredentialError> {
    let valid = !client.client_id.is_empty()
        && client.client_id.len() <= 4096
        && !client.client_secret.is_empty()
        && client.client_secret.expose_secret().len() <= 16 * 1024;
    if valid {
        Ok(())
    } else {
        Err(CredentialError::Provider(Box::new(
            ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("invalid OAuth2 client credentials"),
            ),
        )))
    }
}

fn is_visible_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
}

fn refresh_failure_spec(
    kind: RefreshErrorKind,
    retry: RetryAdvice,
    diagnostic_code: &str,
) -> RefreshFailureSpec {
    let failure = RefreshFailureSpec::new(kind, retry);
    match RefreshDiagnosticCode::parse(diagnostic_code) {
        Ok(code) => failure.with_diagnostic_code(code),
        // A future accidental invalid constant must lose diagnostics rather
        // than turn a proven no-effect response into a less safe outcome.
        Err(_) => failure,
    }
}

/// Build the authorization URL for the Authorization Code grant.
///
/// Appends every query parameter required by RFC 6749 §4.1.1 plus the
/// RFC 7636 PKCE extension and the anti-CSRF `state` parameter.
///
/// Inlined from the former dedicated `oauth2_authorize_url` module so typed
/// pending-state construction remains inside the credential subsystem.
fn build_auth_url(
    config: &OAuth2Config,
    client_id: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, CredentialError> {
    let redirect_uri = config.redirect_uri.as_deref().ok_or_else(|| {
        CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::Schema,
            SecretFreeMessage::new("authorization_code config missing redirect_uri"),
        )))
    })?;
    let pkce_method = config.pkce.ok_or_else(|| {
        CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::Schema,
            SecretFreeMessage::new("authorization_code config missing pkce method"),
        )))
    })?;

    let endpoint = OAuthServerEndpoint::parse(&config.auth_url).map_err(|_| {
        CredentialError::Provider(Box::new(ProviderErrorContext::new(
            ProviderErrorKind::Schema,
            SecretFreeMessage::new("invalid OAuth2 authorization endpoint URL"),
        )))
    })?;
    const OWNED_QUERY_PARAMETERS: [&str; 7] = [
        "response_type",
        "client_id",
        "redirect_uri",
        "scope",
        "state",
        "code_challenge",
        "code_challenge_method",
    ];
    if endpoint
        .expose_url()
        .query_pairs()
        .any(|(key, _)| OWNED_QUERY_PARAMETERS.contains(&key.as_ref()))
    {
        return Err(CredentialError::Provider(Box::new(
            ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("invalid OAuth2 authorization endpoint URL"),
            ),
        )));
    }
    let mut url = endpoint.expose_url().clone();

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

    let url = url.to_string();
    if url.len() > OAUTH_ENDPOINT_MAX_BYTES {
        return Err(CredentialError::Provider(Box::new(
            ProviderErrorContext::new(
                ProviderErrorKind::Schema,
                SecretFreeMessage::new("invalid OAuth2 authorization endpoint URL"),
            ),
        )));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

    use nebula_storage_port::SecretBytes;

    // Trait names referenced only by the tests now that `#[credential]`
    // generates the trait impls via absolute paths: `Credential` (KEY /
    // Properties), `CredentialLifecycle` (policy), and the capability
    // sub-traits exercised by `assert_oauth2_capabilities`.
    use crate::{AuthPattern, Credential, CredentialLifecycle, Interactive, Refreshable};

    use super::*;

    struct FixedAcquisitionTransport {
        status: u16,
        body: &'static [u8],
    }

    impl crate::runtime::AcquisitionTransport for FixedAcquisitionTransport {
        fn post_token<'a>(
            &'a self,
            _request: TokenPostRequest,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            TokenPostResponse,
                            crate::runtime::AcquisitionTransportError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                TokenPostResponse::try_new(self.status, SecretBytes::new(self.body.to_vec()))
                    .map_err(|_| crate::runtime::AcquisitionTransportError::ReadBody)
            })
        }
    }

    fn acquisition_context(status: u16, body: &'static [u8]) -> CredentialContext {
        CredentialContext::for_owner("test-user")
            .for_acquisition(Arc::new(FixedAcquisitionTransport { status, body }))
    }

    fn make_state() -> OAuth2State {
        OAuth2State {
            access_token: SecretString::new("tok_abc"),
            token_type: "Bearer".into(),
            refresh_token: Some(SecretString::new("ref_xyz")),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(3600)),
            scopes: vec!["read".into(), "write".into()],
            client_id: SecretString::new("cid"),
            client_secret: SecretString::new("csecret"),
            token_url: "https://example.com/token?routing=state-url-canary".into(),
            auth_style: AuthStyle::Header,
        }
    }

    #[test]
    fn key_is_oauth2() {
        assert_eq!(OAuth2Credential::KEY, "oauth2");
    }

    #[test]
    fn lifecycle_policy_reflects_refresh_token_presence() {
        // With a refresh token the runtime can renew non-interactively.
        let with_token = make_state();
        let p = OAuth2Credential::policy(&with_token);
        assert_eq!(p.refresh, RefreshStrategy::RefreshToken);
        assert_eq!(p.revoke, RevokeStrategy::None);
        assert!(p.is_auto_renewable());
        assert!(p.is_expiring());

        // Without a refresh token the credential must re-acquire (the refresh
        // path would return ReauthRequired).
        let mut without = make_state();
        without.refresh_token = None;
        let p2 = OAuth2Credential::policy(&without);
        assert_eq!(
            p2.refresh,
            RefreshStrategy::ReAcquire {
                from: None,
                interactive: true
            }
        );
        assert!(!p2.is_auto_renewable());
    }

    // Capability membership names only the implemented provider paths.
    #[expect(dead_code)]
    fn assert_oauth2_capabilities()
    where
        OAuth2Credential: Credential + Interactive + Refreshable,
    {
    }

    #[test]
    fn project_extracts_oauth2_token() {
        let state = make_state();
        let token = OAuth2Credential::project(&state);

        let header = token.bearer_header();
        // bearer_header returns SecretString per §15.5 — exposure happens at
        // the assertion site (test scope), never in production logs.
        assert!(header.expose_secret().contains("tok_abc"));
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
    fn metadata_has_correct_fields() {
        let mut registry = crate::CredentialRegistry::new();
        registry
            .register(OAuth2Credential, "oauth2-metadata-test")
            .expect("valid OAuth2 definition");
        let meta = registry
            .metadata(OAuth2Credential::KEY)
            .expect("registered OAuth2 metadata");
        assert_eq!(meta.pattern(), AuthPattern::OAuth2);
    }

    #[test]
    fn properties_schema_is_admissible() {
        let params = nebula_schema::schema_of::<<OAuth2Credential as Credential>::Properties>()
            .expect("valid OAuth2 schema");
        assert!(!params.fields().is_empty());
    }

    const TEST_CALLBACK: &str = "https://app.example.com/oauth2/callback";

    fn auth_code_pending() -> OAuth2Pending {
        OAuth2Pending {
            config: OAuth2Config::authorization_code(TEST_CALLBACK)
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            auth_style: AuthStyle::Header,
            pkce_verifier: SecretString::new("verifier_value"),
            state: "expected_state".into(),
            redirect_uri: TEST_CALLBACK.into(),
        }
    }

    #[tokio::test]
    async fn continue_resolve_rejects_wrong_input_for_auth_code() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_owner("test-user");
        let result = OAuth2Credential::continue_resolve(&pending, &UserInput::Poll, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_callback_without_code() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_owner("test-user");
        let input = UserInput::Callback {
            params: HashMap::new(),
        };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_callback_missing_state_param() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_owner("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "the_code".to_owned());
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput)));
    }

    #[tokio::test]
    async fn continue_resolve_rejects_wrong_state() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_owner("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "the_code".to_owned());
        params.insert("state".to_owned(), "attacker_state".to_owned());
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput)));
    }

    #[tokio::test]
    async fn continue_resolve_rejects_length_mismatched_state() {
        let mut pending = auth_code_pending();
        pending.state = "aaa".into();
        let ctx = CredentialContext::for_owner("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "c".to_owned());
        params.insert("state".to_owned(), "aaaa".to_owned()); // longer
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput)));
    }

    #[tokio::test]
    async fn continue_resolve_rejects_oversized_callback_before_dispatch() {
        let mut pending = auth_code_pending();
        pending.state = "A".repeat(43);
        let ctx = CredentialContext::for_owner("test-user");
        let input = UserInput::Callback {
            params: [
                ("code".to_owned(), "A".repeat(16 * 1024 + 1)),
                ("state".to_owned(), pending.state.clone()),
            ]
            .into(),
        };

        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;

        assert!(matches!(result, Err(CredentialError::InvalidInput)));
    }

    // ── Authorization-code kickoff coverage ────────────────────────────

    fn auth_code_properties() -> OAuth2AuthorizationCodeProperties {
        OAuth2AuthorizationCodeProperties {
            client: OAuth2ClientProperties {
                client_id: "test_client_id".to_owned(),
                client_secret: SecretString::new("test_client_secret"),
            },
            auth_url: "https://idp.example.com/authorize".to_owned(),
            token_url: "https://idp.example.com/token".to_owned(),
            scopes: Some(vec!["read".to_owned(), "write".to_owned()]),
            redirect_uri: TEST_CALLBACK.to_owned(),
            auth_style: AuthStyle::Header,
        }
    }

    fn client_credentials_properties() -> OAuth2ClientCredentialsProperties {
        OAuth2ClientCredentialsProperties {
            client: OAuth2ClientProperties {
                client_id: "test_client_id".to_owned(),
                client_secret: SecretString::new("test_client_secret"),
            },
            token_url: "https://idp.example.com/token".to_owned(),
            scopes: Some(vec!["read".to_owned(), "write".to_owned()]),
            auth_style: AuthStyle::Header,
        }
    }

    #[tokio::test]
    async fn client_credentials_resolve_completes_through_acquisition_transport() {
        let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
        let ctx = acquisition_context(
            200,
            br#"{"access_token":"access-canary","token_type":"Bearer","expires_in":3600,"scope":"read write"}"#,
        );

        let result = OAuth2Credential::resolve(&properties, &ctx)
            .await
            .expect("client credentials exchange should succeed");
        let StaticResolveResult::Complete(state) = result else {
            panic!("client credentials must complete in one exchange");
        };
        assert_eq!(state.access_token.expose_secret(), "access-canary");
        assert_eq!(state.scopes, ["read", "write"]);
        assert!(!format!("{state:?}").contains("access-canary"));
    }

    #[tokio::test]
    async fn authorization_code_begin_and_callback_complete_through_acquisition_transport() {
        let properties = OAuth2Properties::AuthorizationCode(auth_code_properties());
        let begin =
            OAuth2Credential::begin(&properties, &CredentialContext::for_owner("test-user"))
                .await
                .expect("authorization kickoff should succeed");
        let ResolveResult::Pending { state: pending, .. } = begin else {
            panic!("authorization code must begin with pending state");
        };
        let input = UserInput::Callback {
            params: HashMap::from([
                ("code".to_owned(), "authorization-code".to_owned()),
                ("state".to_owned(), pending.state.clone()),
            ]),
        };
        let ctx = acquisition_context(
            200,
            br#"{"access_token":"callback-access","token_type":"bearer","refresh_token":"callback-refresh","scope":"read"}"#,
        );

        let completed = OAuth2Credential::continue_resolve(&pending, &input, &ctx)
            .await
            .expect("authorization code exchange should succeed");
        let ResolveResult::Complete(state) = completed else {
            panic!("valid callback must complete acquisition");
        };
        assert_eq!(state.access_token.expose_secret(), "callback-access");
        assert_eq!(
            state
                .refresh_token
                .as_ref()
                .expect("provider supplied a refresh token")
                .expose_secret(),
            "callback-refresh"
        );
    }

    #[tokio::test]
    async fn acquisition_without_runtime_transport_fails_structurally() {
        let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
        let error =
            OAuth2Credential::resolve(&properties, &CredentialContext::for_owner("test-user"))
                .await
                .expect_err("initial acquisition authority must be runtime-stamped");
        assert!(matches!(
            error,
            CredentialError::AcquisitionTransportUnavailable
        ));
    }

    #[tokio::test]
    async fn provider_rejection_and_malformed_success_are_payload_free() {
        let properties = OAuth2Properties::ClientCredentials(client_credentials_properties());
        let rejected = OAuth2Credential::resolve(
            &properties,
            &acquisition_context(401, b"provider-secret-canary"),
        )
        .await
        .expect_err("provider rejection must fail");
        assert!(!format!("{rejected:?} {rejected}").contains("provider-secret-canary"));

        let malformed = OAuth2Credential::resolve(
            &properties,
            &acquisition_context(200, b"malformed-success-secret-canary"),
        )
        .await
        .expect_err("malformed success has an unknown provider outcome");
        assert!(matches!(malformed, CredentialError::OutcomeUnknown));
        assert!(!format!("{malformed:?} {malformed}").contains("secret-canary"));
    }

    #[test]
    fn provider_default_scopes_are_accepted_only_within_fixed_bounds() {
        let defaults = SecretString::new("provider.read provider.write");
        assert_eq!(
            parse_granted_scopes(Some(&defaults), &[]).expect("bounded provider defaults"),
            ["provider.read", "provider.write"]
        );

        let too_many = SecretString::new(
            (0..=64)
                .map(|index| format!("scope{index}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        assert!(matches!(
            parse_granted_scopes(Some(&too_many), &[]),
            Err(CredentialError::OutcomeUnknown)
        ));

        let oversized = SecretString::new("s".repeat(257));
        assert!(matches!(
            parse_granted_scopes(Some(&oversized), &[]),
            Err(CredentialError::OutcomeUnknown)
        ));
    }

    #[test]
    fn explicit_scope_request_rejects_unrequested_grants() {
        let returned = SecretString::new("read admin");
        assert!(matches!(
            parse_granted_scopes(Some(&returned), &["read".to_owned()]),
            Err(CredentialError::OutcomeUnknown)
        ));
    }

    #[test]
    fn oauth_properties_keep_client_secret_protected_until_explicit_extraction() {
        let properties = auth_code_properties();
        assert_eq!(
            properties.client.client_secret.expose_secret(),
            "test_client_secret"
        );
        assert!(!format!("{:?}", properties.client.client_secret).contains("test_client_secret"));
    }

    #[tokio::test]
    async fn initiate_authorization_code_returns_redirect_with_pkce_and_state() {
        let properties = auth_code_properties();
        let (pending, request) =
            initiate_authorization_code(&properties).expect("kickoff should succeed");

        let url = match request {
            InteractionRequest::Redirect { url } => url,
            other => panic!("expected Redirect, got {other:?}"),
        };

        // RFC 6749 §4.1.1 + RFC 7636 PKCE — mandatory query parameters.
        assert!(url.contains("response_type=code"), "missing response_type");
        assert!(url.contains("client_id="), "missing client_id");
        assert!(url.contains("redirect_uri="), "missing redirect_uri");
        assert!(url.contains("scope="), "missing scope");
        assert!(url.contains("state="), "missing state");
        assert!(url.contains("code_challenge="), "missing code_challenge");
        assert!(
            url.contains("code_challenge_method=S256"),
            "missing or wrong code_challenge_method"
        );

        // Pending state populated for AuthorizationCode flow per §15.4.
        assert!(!pending.pkce_verifier.is_empty());
        assert!(!pending.state.is_empty());
        assert_eq!(pending.redirect_uri, TEST_CALLBACK);
    }

    #[tokio::test]
    async fn initiate_authorization_code_csrf_state_is_unguessable() {
        let properties = auth_code_properties();
        let (pending1, _) = initiate_authorization_code(&properties).expect("first kickoff");
        let (pending2, _) = initiate_authorization_code(&properties).expect("second kickoff");

        let state1 = &pending1.state;
        let state2 = &pending2.state;

        assert_ne!(
            state1, state2,
            "anti-CSRF state must be unguessable across kickoffs"
        );

        // `generate_random_state` produces ≥128 bits of base64-encoded
        // entropy → at least 22 base64 chars.
        assert!(
            state1.len() >= 22,
            "state token should carry ≥128 bits of entropy: got {} chars",
            state1.len()
        );
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
            auth_style: AuthStyle::Header,
        };

        let ctx = CredentialContext::for_owner("test-user");
        let outcome = OAuth2Credential::refresh(
            &mut state,
            RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
        )
        .await
        .into_kind();
        // Locally detected: never spoke to the IdP. Distinct from
        // `ProviderRejected` per wave-2 review (see ReauthReason rustdoc).
        assert!(matches!(
            outcome,
            crate::contract::RefreshReportKind::ReauthRequired {
                reason: crate::resolve::ReauthReason::MissingRefreshMaterial,
                phase: crate::contract::RefreshReauthPhase::BeforeDispatch,
            }
        ));
    }

    #[tokio::test]
    async fn refresh_without_runtime_transport_is_exact_and_bounded_retryable() {
        let mut state = make_state();
        let ctx = CredentialContext::for_owner("test-user");
        let report = OAuth2Credential::refresh(
            &mut state,
            RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider),
        )
        .await
        .into_kind();

        let crate::contract::RefreshReportKind::NotApplied(context) = report else {
            panic!("missing runtime transport must be a proven not-applied refresh");
        };
        assert_eq!(
            context.phase(),
            crate::RefreshNotAppliedPhase::BeforeDispatch
        );
        assert_eq!(context.kind(), RefreshErrorKind::ProviderUnavailable);
        let RetryAdvice::After(delay) = context.retry() else {
            panic!("runtime composition can recover without a credential material update");
        };
        assert_eq!(delay.get(), Duration::from_mins(1));
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
            auth_style: AuthStyle::Header,
        };
        // Expires in 30s, margin is 60s => expired
        assert!(state.is_expired(Duration::from_mins(1)));
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
            auth_style: AuthStyle::Header,
        };
        assert!(!state.is_expired(Duration::from_secs(9999)));
    }

    #[test]
    fn pending_state_zeroizes_all_fields_including_pkce_verifier_state_redirect() {
        let mut pending = OAuth2Pending {
            config: OAuth2Config::authorization_code(TEST_CALLBACK)
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("super_secret"),
            auth_style: AuthStyle::Header,
            pkce_verifier: SecretString::new("verifier_contents"),
            state: "state_contents".into(),
            redirect_uri: TEST_CALLBACK.into(),
        };

        pending.zeroize();
        assert!(pending.config.auth_url.is_empty());
        assert!(pending.config.token_url.is_empty());
        assert!(pending.config.scopes.is_empty());
        assert!(pending.config.redirect_uri.is_none());
        assert!(pending.client_secret.expose_secret().is_empty());
        assert!(pending.client_id.is_empty());
        assert!(pending.pkce_verifier.expose_secret().is_empty());
        assert!(pending.state.is_empty());
        assert!(pending.redirect_uri.is_empty());
    }

    #[test]
    fn pending_state_debug_is_constant_and_redacts_all_urls() {
        let first = OAuth2Pending {
            config: OAuth2Config::authorization_code(TEST_CALLBACK)
                .auth_url("https://a.com/auth?diagnostic=auth-url-canary")
                .token_url("https://a.com/token?diagnostic=token-url-canary")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            auth_style: AuthStyle::Header,
            pkce_verifier: SecretString::new("my_pkce_verifier_value"),
            state: "my_csrf_state_value".into(),
            redirect_uri: TEST_CALLBACK.into(),
        };
        let second = OAuth2Pending {
            config: OAuth2Config::authorization_code("https://different.example/callback")
                .auth_url("https://different.example/long/authorize/path")
                .token_url("https://different.example/token")
                .build(),
            client_id: "different-client".into(),
            client_secret: SecretString::new("different-secret"),
            auth_style: AuthStyle::PostBody,
            pkce_verifier: SecretString::new("different-verifier"),
            state: "different-state".into(),
            redirect_uri: "https://different.example/callback".into(),
        };
        let debug = format!("{first:?}");
        assert_eq!(debug, format!("{second:?}"));
        assert_eq!(debug, "OAuth2Pending(<redacted>)");
        for canary in [
            "my_pkce_verifier_value",
            "my_csrf_state_value",
            "auth-url-canary",
            "token-url-canary",
            TEST_CALLBACK,
        ] {
            assert!(!debug.contains(canary));
        }
    }

    #[test]
    fn pending_state_expires_in_10_minutes() {
        let pending = OAuth2Pending {
            config: OAuth2Config::authorization_code(TEST_CALLBACK)
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            auth_style: AuthStyle::Header,
            pkce_verifier: SecretString::new("verifier"),
            state: "state".into(),
            redirect_uri: TEST_CALLBACK.into(),
        };

        assert_eq!(pending.expires_in(), Duration::from_mins(10));
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
        // bearer_header returns SecretString per §15.5 — exposure happens at
        // the assertion site (test scope), never in production logs.
        assert_eq!(state.bearer_header().expose_secret(), "Bearer tok_abc");
    }

    #[test]
    fn oauth2_state_debug_redacts_secrets() {
        let state = make_state();
        let debug = format!("{state:?}");
        assert!(!debug.contains("tok_abc"), "access_token leaked in Debug");
        assert!(!debug.contains("ref_xyz"), "refresh_token leaked in Debug");
        assert!(!debug.contains("csecret"), "client_secret leaked in Debug");
        assert!(
            !debug.contains("state-url-canary"),
            "query-bearing token_url leaked in Debug"
        );
        assert!(!debug.contains("example.com"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("Bearer"));
    }

    #[test]
    fn oauth2_state_zeroize_scrubs_token_url() {
        let mut state = make_state();
        state.zeroize();
        assert!(state.token_url.is_empty());
    }

    #[test]
    fn build_auth_url_parse_failure_is_fixed_and_input_free() {
        let config = OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url("://auth-url-diagnostic-canary")
            .token_url("https://provider.example/token")
            .build();

        let error = build_auth_url(&config, "client", "challenge", "state")
            .expect_err("invalid authorization endpoint must fail");
        let diagnostic = format!("{error:?} {error}");
        assert!(!diagnostic.contains("auth-url-diagnostic-canary"));
        assert!(!diagnostic.contains("://"));
        assert!(diagnostic.contains("invalid OAuth2 authorization endpoint URL"));
    }

    #[test]
    fn build_auth_url_rejects_endpoints_outside_oauth_egress_policy() {
        for auth_url in [
            "http://provider.example/authorize",
            "https://localhost/authorize",
            "https://127.0.0.1/authorize",
            "https://user@provider.example/authorize",
            "https://provider.example/authorize#fragment",
            "https://provider.example/authorize?state=attacker",
        ] {
            let config = OAuth2Config::authorization_code(TEST_CALLBACK)
                .auth_url(auth_url)
                .token_url("https://provider.example/token")
                .build();

            let result = build_auth_url(&config, "client", "challenge", "state");
            assert!(
                result.is_err(),
                "accepted authorization endpoint {auth_url}"
            );
        }
    }

    #[test]
    fn build_auth_url_rejects_an_oversized_final_redirect() {
        let config = OAuth2Config::authorization_code(TEST_CALLBACK)
            .auth_url("https://provider.example/authorize")
            .token_url("https://provider.example/token")
            .scopes(["a".repeat(8 * 1024)])
            .build();

        let result = build_auth_url(&config, "client", "challenge", "state");
        assert!(result.is_err(), "accepted an oversized authorization URL");
    }

    #[test]
    fn oauth2_state_serde_round_trip() {
        let state = make_state();

        // Default sink (logs, responses): every secret field redacts.
        let redacted = serde_json::to_string(&state).unwrap();
        assert!(
            !redacted.contains("tok_abc"),
            "access_token leaked to default serde sink"
        );
        assert!(
            !redacted.contains("ref_xyz"),
            "refresh_token leaked to default serde sink"
        );
        assert!(
            !redacted.contains("csecret"),
            "client_secret leaked to default serde sink"
        );

        // Storage scope preserves them for encrypted-at-rest persistence.
        let json = crate::serde_secret::expose_for_serialization(|| serde_json::to_string(&state))
            .unwrap();
        let restored: OAuth2State = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.access_token.expose_secret(), "tok_abc");
        assert_eq!(
            restored.refresh_token.as_ref().unwrap().expose_secret(),
            "ref_xyz"
        );
        assert_eq!(restored.client_id.expose_secret(), "cid");
        assert_eq!(restored.client_secret.expose_secret(), "csecret");
    }
}
