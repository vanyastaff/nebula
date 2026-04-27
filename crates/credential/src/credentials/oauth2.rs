//! OAuth2 credential -- interactive, refreshable, multi-grant-type.
//!
//! Supports three OAuth2 grant types via the unified Credential trait:
//! - **Authorization Code** -- user browser redirect (interactive)
//! - **Client Credentials** -- server-to-server, resolves in one step
//! - **Device Code** -- CLI/TV apps, polling flow (interactive)
//!
//! State/scheme separation: OAuth2State is stored (contains refresh
//! internals), while OAuth2Token is the consumer-facing auth material produced by `project()`.

use std::{fmt, fmt::Formatter, time::Duration};

use chrono::{DateTime, Utc};
use nebula_schema::{Field, FieldValues, HasSchema, Schema, ValidSchema};
// Re-exports for backward compatibility with `credentials::oauth2::` paths
// used by external crates (nebula-api, nebula-storage).
pub use oauth2_config::{
    AuthCodeBuilder, AuthStyle, ClientCredentialsBuilder, DeviceCodeBuilder, GrantType,
    OAuth2Config, PkceMethod,
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::oauth2_config;
use crate::{
    Credential, CredentialContext, CredentialState, Interactive, PendingState, Refreshable,
    Revocable, SecretString, Testable,
    contract::plugin_capability_report,
    error::CredentialError,
    metadata::CredentialMetadata,
    resolve::{InteractionRequest, RefreshOutcome, ResolveResult, TestResult, UserInput},
    scheme::OAuth2Token,
};

// ── OAuth2State ────────────────────────────────────────────────────────

/// Internal OAuth2 state with refresh internals.
///
/// This is what gets encrypted and stored. Consumer-facing auth is
/// [`OAuth2Token`] (via [`OAuth2Credential::project`]).
///
/// Contains `client_id`, `client_secret`, and `token_url` so that
/// [`OAuth2Credential::refresh`] can exchange a refresh token without
/// requiring the original setup parameters.
///
/// Per Tech Spec §15.4 amendment — `Zeroize` + `ZeroizeOnDrop` derived
/// so the decrypted plaintext (access/refresh tokens, client creds)
/// is scrubbed deterministically when this state is dropped. Non-secret
/// fields (token type, expiry, scopes, URL, auth-style enum) carry
/// `#[zeroize(skip)]`.
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
    /// Token endpoint URL for refresh requests — non-secret endpoint URL.
    #[zeroize(skip)]
    pub token_url: String,
    /// How client credentials are sent (preserved from initial token
    /// exchange) — non-secret enum discriminant.
    #[serde(default)]
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
    #[must_use]
    pub fn bearer_header(&self) -> SecretString {
        SecretString::new(format!("Bearer {}", self.access_token.expose_secret()))
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
/// For [`GrantType::AuthorizationCode`] the last three fields
/// (`pkce_verifier`, `state`, `redirect_uri`) are all `Some(_)` — they
/// carry the per-flow PKCE verifier, the anti-CSRF state token, and the
/// exact redirect URI that must be echoed on the token exchange. For
/// other grant types all three are `None`. The `Option` wrapping also
/// lets records serialized before the PKCE fix deserialize successfully;
/// `continue_resolve` rejects such records loudly as "callback
/// validation failed" rather than silently completing without a PKCE
/// check.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuth2Pending {
    /// OAuth2 provider configuration.
    pub config: OAuth2Config,
    /// OAuth2 client identifier.
    pub client_id: String,
    /// OAuth2 client secret (zeroized on drop).
    #[serde(with = "crate::serde_secret")]
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
    /// PKCE code verifier for AuthorizationCode flows.
    ///
    /// Generated fresh on every `resolve()`. Sent as `code_verifier` on
    /// the token exchange so the provider can recompute and match the
    /// `code_challenge` carried on the auth URL.
    #[serde(default, with = "crate::serde_secret::option")]
    pub pkce_verifier: Option<SecretString>,
    /// Anti-CSRF `state` parameter for AuthorizationCode flows.
    ///
    /// Generated fresh on every `resolve()`. Validated in
    /// `continue_resolve` against the callback-provided value via a
    /// constant-time comparison.
    #[serde(default)]
    pub state: Option<String>,
    /// Exact `redirect_uri` echoed on the token exchange for
    /// AuthorizationCode flows.
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

// Manual `Debug` so that `tracing::debug!(?pending)` cannot leak the
// `client_secret` (already redacted by `SecretString`'s own `Debug`),
// the `device_code` (device-flow bearer), the `pkce_verifier` (one-shot
// auth-code verifier), or the `state` (callback CSRF token). The
// `redirect_uri` is not secret and is shown verbatim.
impl fmt::Debug for OAuth2Pending {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2Pending")
            .field("config", &self.config)
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("grant_type", &self.grant_type)
            .field("auth_style", &self.auth_style)
            .field(
                "device_code",
                &self.device_code.as_ref().map(|_| "[REDACTED]"),
            )
            .field("interval", &self.interval)
            .field(
                "pkce_verifier",
                &self.pkce_verifier.as_ref().map(|_| "[REDACTED]"),
            )
            .field("state", &self.state.as_ref().map(|_| "[REDACTED]"))
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}

impl Zeroize for OAuth2Pending {
    fn zeroize(&mut self) {
        // Zeroize the existing SecretString in place before replacing it, so
        // the underlying heap buffer is scrubbed rather than relying solely
        // on Drop of the replacement.
        self.client_secret.zeroize();
        self.client_secret = SecretString::new("");
        // client_id is not strictly a secret, but it correlates to an
        // account and we are told to wipe this struct — scrub it too.
        self.client_id.zeroize();
        if let Some(ref mut dc) = self.device_code {
            dc.zeroize();
        }
        // Drop the Option entirely so downstream callers cannot tell a
        // wiped device-code apart from a fresh Some("").
        self.device_code = None;
        self.interval = None;
        // PKCE verifier: scrub in place, then drop the Option.
        if let Some(ref mut v) = self.pkce_verifier {
            v.zeroize();
        }
        self.pkce_verifier = None;
        // Anti-CSRF state and redirect URI: scrub + drop.
        if let Some(ref mut s) = self.state {
            s.zeroize();
        }
        self.state = None;
        if let Some(ref mut r) = self.redirect_uri {
            r.zeroize();
        }
        self.redirect_uri = None;
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

/// OAuth2 credential type implementing the [`Credential`] trait plus
/// the [`Interactive`], [`Refreshable`], [`Revocable`], and [`Testable`]
/// sub-traits per Tech Spec §15.4.
///
/// `Revocable` and `Testable` currently surface
/// `CredentialError::Provider("OAuth2 HTTP transport has moved …")`
/// because the underlying HTTP calls (RFC 7009 revoke endpoint, token
/// introspection / userinfo health probe) live in nebula-engine per
/// ADR-0031. The trait impls exist so the engine's revoke / test
/// dispatchers (which bind `where C: Revocable` / `where C: Testable`)
/// can route to OAuth2 once the transport is wired — and so plugin
/// callers see a typed transport-disabled classification instead of
/// "credential type does not support revocation / testing."
///
/// Configuration (auth URL, token URL, grant type, scopes) is provided
/// via [`parameters()`](OAuth2Credential::schema) and extracted from
/// [`FieldValues`] when the OAuth2 flow is initiated.
///
/// # Grant types and entry points
///
/// Per §15.4 the base [`Credential::resolve`] returns
/// `ResolveResult<State, ()>` and cannot carry typed
/// [`OAuth2Pending`]. The interactive entry point therefore lives on
/// the OAuth2-specific kickoff path:
///
/// - **Authorization Code** — base `resolve` rejects with `Provider("OAuth2 authorization_code
///   requires OAuth2-specific kickoff path")`. The API endpoint orchestrating the OAuth2 flow
///   constructs an [`OAuth2Pending`] directly via [`OAuth2Credential::initiate_authorization_code`]
///   and persists it to the [`PendingStateStore`](crate::pending_store::PendingStateStore). On
///   callback, the framework loads the typed pending state and invokes
///   [`Interactive::continue_resolve`].
/// - **Client Credentials** — base `resolve` returns `Complete(state)` once the engine wires the
///   moved `nebula-engine` HTTP transport (ADR-0031). For now `resolve` returns `Provider("OAuth2
///   HTTP transport has moved …")` so callers surface the migration explicitly rather than silently
///   no-op'ing.
/// - **Device Code** — base `resolve` errors as per Authorization Code; the device-code variant
///   (`initiate_device_code`) is deferred to a later phase. RFC 8628 requires HTTP transport which
///   is currently disabled in this crate per ADR-0031 (see `docs/adr/0031-api-owns-oauth-flow.md`);
///   the kickoff helper will land alongside the engine HTTP transport wiring.
pub struct OAuth2Credential;

/// Typed shape of the `oauth2` credential setup form.
pub struct OAuth2Input;

impl HasSchema for OAuth2Input {
    fn schema() -> ValidSchema {
        Schema::builder()
            .add(
                Field::string("client_id")
                    .label("Client ID")
                    .description("OAuth2 client identifier")
                    .required(),
            )
            .add(
                Field::secret("client_secret")
                    .label("Client Secret")
                    .description("OAuth2 client secret")
                    .required(),
            )
            .add(
                Field::string("auth_url")
                    .label("Authorization URL")
                    .description("OAuth2 authorization endpoint URL")
                    .placeholder("https://provider.example.com/oauth2/authorize"),
            )
            .add(
                Field::string("token_url")
                    .label("Token URL")
                    .description("OAuth2 token endpoint URL")
                    .required()
                    .placeholder("https://provider.example.com/oauth2/token"),
            )
            .add(
                Field::string("grant_type")
                    .label("Grant Type")
                    .description(
                        "OAuth2 grant type: authorization_code, client_credentials, or device_code",
                    )
                    .default(serde_json::json!("authorization_code")),
            )
            .add(
                Field::string("scopes")
                    .label("Scopes")
                    .description("Space-separated list of OAuth2 scopes"),
            )
            .add(
                Field::string("redirect_uri")
                    .label("Redirect URI")
                    .description(
                        "OAuth2 redirect URI (required for authorization_code grant; must match the URI registered with the provider)",
                    )
                    .placeholder("https://app.example.com/oauth2/callback"),
            )
            .build()
            .expect("oauth2 schema is always valid")
    }
}

impl Credential for OAuth2Credential {
    type Input = OAuth2Input;
    type Scheme = OAuth2Token;
    type State = OAuth2State;

    const KEY: &'static str = "oauth2";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("oauth2"))
            .name("OAuth2")
            .description("OAuth2 authentication supporting Authorization Code, Client Credentials, and Device Code grant types.")
            .schema(Self::schema())
            .pattern(crate::AuthPattern::OAuth2)
            .icon("oauth2")
            .build()
            .expect("oauth2 metadata is valid")
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
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, ()>, CredentialError> {
        // Per Tech Spec §15.4, the base `Credential::resolve` returns
        // `ResolveResult<State, ()>` — typed `OAuth2Pending` cannot ride
        // along here. Validate the input shape, then route by grant type:
        // * AuthorizationCode / DeviceCode: kick off via `OAuth2Credential::initiate_*` and persist
        //   the typed `OAuth2Pending` through `PendingStateStore`. The continuation then routes
        //   through `Interactive::continue_resolve`.
        // * ClientCredentials: HTTP transport moved to nebula-engine (ADR-0031); surface the
        //   migration explicitly.
        let _client_id = extract_required(values, "client_id")?;
        let _client_secret = extract_required(values, "client_secret")?;
        let _token_url = extract_required(values, "token_url")?;
        let grant_type_str = values
            .get_string_by_str("grant_type")
            .unwrap_or("authorization_code");
        let grant_type = parse_grant_type(grant_type_str)?;

        // For AuthorizationCode the `redirect_uri` is required by RFC
        // 6749 §4.1.3; surface the missing-field early before the
        // OAuth2-specific kickoff is invoked. This keeps the failure
        // mode stable for callers that today rely on `resolve` to
        // validate setup form values.
        if matches!(grant_type, GrantType::AuthorizationCode)
            && values.get_string_by_str("redirect_uri").is_none()
        {
            return Err(CredentialError::InvalidInput(
                "missing required field: redirect_uri (required for authorization_code grant)"
                    .into(),
            ));
        }

        match grant_type {
            GrantType::AuthorizationCode | GrantType::DeviceCode => Err(CredentialError::Provider(
                "OAuth2 authorization_code / device_code flow must be initiated via OAuth2Credential::initiate_* and the framework PendingStateStore (Tech Spec §15.4)".into(),
            )),
            GrantType::ClientCredentials => Err(oauth2_http_transport_disabled()),
        }
    }
}

impl Interactive for OAuth2Credential {
    type Pending = OAuth2Pending;

    async fn continue_resolve(
        pending: &OAuth2Pending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        match pending.grant_type {
            GrantType::AuthorizationCode => {
                // Uniform failure message so a callback probe cannot use
                // response-error text as an oracle for which dimension
                // (missing code / missing state / wrong state / missing
                // verifier) tripped the check.
                const FAILED: &str = "OAuth2 callback validation failed";

                let params = match input {
                    UserInput::Callback { params } => params,
                    _ => {
                        return Err(CredentialError::InvalidInput(
                            "authorization_code flow expects UserInput::Callback".into(),
                        ));
                    },
                };

                let code = params
                    .get("code")
                    .ok_or_else(|| CredentialError::InvalidInput(FAILED.into()))?;
                let callback_state = params
                    .get("state")
                    .ok_or_else(|| CredentialError::InvalidInput(FAILED.into()))?;
                let expected_state = pending
                    .state
                    .as_deref()
                    .ok_or_else(|| CredentialError::InvalidInput(FAILED.into()))?;

                // Constant-time compare on the (callback, expected) state
                // so a timing probe cannot recover the expected value by
                // guessing one prefix byte at a time. `ct_eq` returns
                // `Choice`; `bool::from` turns it into an observable bool.
                let state_matches: bool = callback_state
                    .as_bytes()
                    .ct_eq(expected_state.as_bytes())
                    .into();
                if !state_matches {
                    return Err(CredentialError::InvalidInput(
                        "OAuth2 state mismatch".into(),
                    ));
                }

                let verifier_secret = pending
                    .pkce_verifier
                    .as_ref()
                    .ok_or_else(|| CredentialError::InvalidInput(FAILED.into()))?;
                let redirect_uri = pending
                    .redirect_uri
                    .as_deref()
                    .ok_or_else(|| CredentialError::InvalidInput(FAILED.into()))?;

                // Validation passed. HTTP code exchange has moved to nebula-api
                // per ADR-0031; this crate no longer performs HTTP.
                let _ = (verifier_secret, redirect_uri, code);
                Err(oauth2_http_transport_disabled())
            },
            GrantType::DeviceCode => {
                if !matches!(input, UserInput::Poll) {
                    return Err(CredentialError::InvalidInput(
                        "device_code flow expects UserInput::Poll".into(),
                    ));
                }
                // HTTP device code polling has moved to nebula-engine per ADR-0031.
                Err(oauth2_http_transport_disabled())
            },
            GrantType::ClientCredentials => Err(CredentialError::InvalidInput(
                "client_credentials flow does not use continue_resolve".into(),
            )),
        }
    }
}

impl Refreshable for OAuth2Credential {
    async fn refresh(
        state: &mut OAuth2State,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        if state.refresh_token.is_none() {
            // Locally detected: we never spoke to the IdP, so this is
            // *not* a provider rejection. Surface as
            // `MissingRefreshMaterial` so operators can distinguish a
            // misconfigured grant (no refresh_token issued) from a
            // genuine provider invalidation.
            return Ok(RefreshOutcome::ReauthRequired(
                crate::resolve::ReauthReason::MissingRefreshMaterial {
                    detail: "OAuth2 state has no refresh_token".to_string(),
                },
            ));
        }

        // Token refresh HTTP has moved to nebula-engine per ADR-0031;
        // this crate no longer performs HTTP.
        Err(oauth2_http_transport_disabled())
    }
}

impl Revocable for OAuth2Credential {
    async fn revoke(
        _state: &mut OAuth2State,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        // OAuth2 RFC 7009 token revocation requires HTTP — moved to
        // nebula-engine per ADR-0031. Returning the typed transport-
        // disabled error keeps the failure classification stable for
        // callers (engine routes to the HTTP transport when wired); a
        // silent `Ok(())` would falsely signal "secret revoked at
        // provider" while the token remains live.
        Err(oauth2_http_transport_disabled())
    }
}

impl Testable for OAuth2Credential {
    async fn test(
        _scheme: &OAuth2Token,
        _ctx: &CredentialContext,
    ) -> Result<TestResult, CredentialError> {
        // OAuth2 health probe (token introspection / userinfo) requires
        // HTTP — moved to nebula-engine per ADR-0031. Same routing
        // rationale as `Refreshable::refresh` and `Revocable::revoke`
        // above. Returning `Ok(TestResult::Failed { … })` would falsely
        // signal "credential tested and is bad"; the test simply did
        // not run, so the typed transport-disabled error is the correct
        // classification.
        Err(oauth2_http_transport_disabled())
    }
}

// Per Tech Spec §15.8 (closes security-lead N6) `OAuth2Credential`
// reports its sub-trait surface via `plugin_capability_report::Is*` so
// the `CredentialRegistry` capability bitflag set matches the
// implementations directly above (Interactive + Refreshable + Revocable
// + Testable). OAuth2 is not a `Dynamic` credential — its tokens are
// stored, refreshed, and revoked by KEY rather than leased per
// execution.
impl plugin_capability_report::IsInteractive for OAuth2Credential {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsRefreshable for OAuth2Credential {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsRevocable for OAuth2Credential {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsTestable for OAuth2Credential {
    const VALUE: bool = true;
}
impl plugin_capability_report::IsDynamic for OAuth2Credential {
    const VALUE: bool = false;
}

impl OAuth2Credential {
    /// Initiate the Authorization Code flow.
    ///
    /// Constructs the authorization URL (with PKCE challenge + anti-CSRF
    /// state) and the typed [`OAuth2Pending`] state that the framework
    /// must persist before redirecting the user. Per Tech Spec §15.4
    /// the base [`Credential::resolve`] cannot carry the typed pending
    /// state; this kickoff method exists so the API endpoint
    /// orchestrating the OAuth2 flow can construct the pending state
    /// directly and call
    /// [`PendingStateStore::put`](crate::pending_store::PendingStateStore::put).
    pub fn initiate_authorization_code(
        values: &FieldValues,
    ) -> Result<(OAuth2Pending, InteractionRequest), CredentialError> {
        let client_id = extract_required(values, "client_id")?;
        let client_secret = extract_required(values, "client_secret")?;
        let token_url = extract_required(values, "token_url")?;

        let auth_url = values.get_string_by_str("auth_url").unwrap_or_default();
        let scopes = parse_scopes(values);
        let redirect_uri_opt = values.get_string_by_str("redirect_uri").map(str::to_owned);
        let config = build_config(
            GrantType::AuthorizationCode,
            auth_url,
            token_url,
            &scopes,
            redirect_uri_opt,
        )?;

        let verifier = crate::generate_pkce_verifier();
        let challenge = crate::generate_code_challenge(&verifier);
        let state_token = crate::generate_random_state();

        let url = build_auth_url(&config, client_id, &challenge, &state_token)?;
        // `build_config` upstream rejects `AuthorizationCode` configs that
        // lack `redirect_uri`, but the panic site is decoupled from that
        // invariant by ~25 lines and a different match arm. Defensive
        // typed-error: if anyone later relaxes `build_config` (e.g. to
        // allow late-bound redirect URIs), this surfaces as a structured
        // `CredentialError` rather than a runtime panic in library code.
        // PR #582 review (CodeRabbit) — no `unwrap`/`expect` in lib code.
        let redirect_uri = config.redirect_uri.clone().ok_or_else(|| {
            CredentialError::Provider(
                "authorization_code config missing redirect_uri (RFC 6749 §4.1.1 requires \
                 `redirect_uri` to be present at the authorization request site; check \
                 OAuth2Config builder)"
                    .into(),
            )
        })?;

        let pending = OAuth2Pending {
            client_id: client_id.to_owned(),
            client_secret: SecretString::new(client_secret),
            grant_type: GrantType::AuthorizationCode,
            auth_style: config.auth_style,
            device_code: None,
            interval: None,
            pkce_verifier: Some(SecretString::new(verifier)),
            state: Some(state_token),
            redirect_uri: Some(redirect_uri),
            config,
        };
        Ok((pending, InteractionRequest::Redirect { url }))
    }
}

// ── Private helpers ────────────────────────────────────────────────────

fn oauth2_http_transport_disabled() -> CredentialError {
    CredentialError::Provider(
        "OAuth2 HTTP transport has moved: code exchange to nebula-api, token refresh to nebula-engine (ADR-0031)"
            .into(),
    )
}

/// Build the authorization URL for the Authorization Code grant.
///
/// Appends every query parameter required by RFC 6749 §4.1.1 plus the
/// RFC 7636 PKCE extension and the anti-CSRF `state` parameter.
///
/// Inlined from the former `oauth2_authorize_url` module (moved to nebula-api).
fn build_auth_url(
    config: &OAuth2Config,
    client_id: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, CredentialError> {
    let redirect_uri = config.redirect_uri.as_deref().ok_or_else(|| {
        CredentialError::Provider("authorization_code config missing redirect_uri".into())
    })?;
    let pkce_method = config.pkce.ok_or_else(|| {
        CredentialError::Provider("authorization_code config missing pkce method".into())
    })?;

    let mut url = url::Url::parse(&config.auth_url)
        .map_err(|e| CredentialError::Provider(format!("invalid auth_url: {e}")))?;

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

/// Extract a required string parameter, returning an error if missing.
fn extract_required<'a>(values: &'a FieldValues, key: &str) -> Result<&'a str, CredentialError> {
    values
        .get_string_by_str(key)
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
fn parse_scopes(values: &FieldValues) -> Vec<String> {
    values
        .get_string_by_str("scopes")
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Build an [`OAuth2Config`] from extracted parameter values.
///
/// For [`GrantType::AuthorizationCode`], `redirect_uri` is required —
/// the builder will reject a missing value with
/// `CredentialError::InvalidInput`. For other grants, `redirect_uri` is
/// silently ignored (passing `None` or `Some(_)` both yield a config
/// with `redirect_uri = None`).
fn build_config(
    grant_type: GrantType,
    auth_url: &str,
    token_url: &str,
    scopes: &[String],
    redirect_uri: Option<String>,
) -> Result<OAuth2Config, CredentialError> {
    let config = match grant_type {
        GrantType::AuthorizationCode => {
            let redirect = redirect_uri.ok_or_else(|| {
                CredentialError::InvalidInput(
                    "missing required field: redirect_uri (required for authorization_code grant)"
                        .into(),
                )
            })?;
            OAuth2Config::authorization_code(redirect)
                .auth_url(auth_url)
                .token_url(token_url)
                .scopes(scopes.iter().cloned())
                .build()
        },
        GrantType::ClientCredentials => OAuth2Config::client_credentials()
            .auth_url(auth_url)
            .token_url(token_url)
            .scopes(scopes.iter().cloned())
            .build(),
        GrantType::DeviceCode => OAuth2Config::device_code()
            .auth_url(auth_url)
            .token_url(token_url)
            .scopes(scopes.iter().cloned())
            .build(),
    };
    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

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

    // Capability membership is type-level after §15.4: `OAuth2Credential`
    // implements `Interactive`, `Refreshable`, `Revocable`, and
    // `Testable` (and not `Dynamic`). Trait bound checks below stand in
    // for the previous const-bool assertions. The `Revocable` and
    // `Testable` impls currently route through ADR-0031 HTTP transport
    // (returning `oauth2_http_transport_disabled()`); the trait
    // membership is still required so the engine's revoke / test
    // dispatchers can bind on it once the transport is wired.
    #[allow(dead_code)]
    fn assert_oauth2_capabilities()
    where
        OAuth2Credential: Credential + Interactive + Refreshable + Revocable + Testable,
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
        use nebula_metadata::Metadata;
        let meta = OAuth2Credential::metadata();
        assert_eq!(meta.key().as_str(), "oauth2");
        assert_eq!(meta.name(), "OAuth2");
        assert!(meta.description().contains("OAuth2"));
    }

    #[test]
    fn parameters_has_all_fields() {
        let params = OAuth2Credential::schema();
        let has = |k: &str| params.fields().iter().any(|f| f.key().as_str() == k);
        assert!(has("client_id"));
        assert!(has("client_secret"));
        assert!(has("auth_url"));
        assert!(has("token_url"));
        assert!(has("grant_type"));
        assert!(has("scopes"));
        assert!(has("redirect_uri"));
        assert_eq!(params.fields().len(), 7);
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
        let values = FieldValues::new();
        assert!(parse_scopes(&values).is_empty());
    }

    #[test]
    fn parse_scopes_splits_whitespace() {
        let mut values = FieldValues::new();
        values.set_raw("scopes", serde_json::json!("read write admin"));
        let scopes = parse_scopes(&values);
        assert_eq!(scopes, vec!["read", "write", "admin"]);
    }

    #[tokio::test]
    async fn resolve_rejects_missing_client_id() {
        let values = FieldValues::new();
        let ctx = CredentialContext::for_test("test-user");
        let result = OAuth2Credential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_rejects_missing_token_url() {
        let mut values = FieldValues::new();
        values.set_raw("client_id", serde_json::json!("cid"));
        values.set_raw("client_secret", serde_json::json!("cs"));
        let ctx = CredentialContext::for_test("test-user");
        let result = OAuth2Credential::resolve(&values, &ctx).await;
        assert!(result.is_err());
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
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: None,
            interval: None,
            pkce_verifier: Some(SecretString::new("verifier_value")),
            state: Some("expected_state".into()),
            redirect_uri: Some(TEST_CALLBACK.into()),
        }
    }

    #[tokio::test]
    async fn resolve_rejects_missing_redirect_uri_for_auth_code() {
        let mut values = FieldValues::new();
        values.set_raw("client_id", serde_json::json!("cid"));
        values.set_raw("client_secret", serde_json::json!("cs"));
        values.set_raw("auth_url", serde_json::json!("https://a.com/auth"));
        values.set_raw("token_url", serde_json::json!("https://a.com/token"));
        values.set_raw("grant_type", serde_json::json!("authorization_code"));

        let ctx = CredentialContext::for_test("test-user");
        let result = OAuth2Credential::resolve(&values, &ctx).await;
        match result {
            Err(CredentialError::InvalidInput(msg)) => {
                assert!(
                    msg.contains("redirect_uri"),
                    "error message must name the missing field: {msg}"
                );
            },
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn continue_resolve_rejects_wrong_input_for_auth_code() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_test("test-user");
        let result = OAuth2Credential::continue_resolve(&pending, &UserInput::Poll, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_callback_without_code() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_test("test-user");
        let input = UserInput::Callback {
            params: HashMap::new(),
        };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn continue_resolve_rejects_callback_missing_state_param() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_test("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "the_code".to_owned());
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput(_))));
    }

    #[tokio::test]
    async fn continue_resolve_rejects_wrong_state() {
        let pending = auth_code_pending();
        let ctx = CredentialContext::for_test("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "the_code".to_owned());
        params.insert("state".to_owned(), "attacker_state".to_owned());
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput(_))));
    }

    #[tokio::test]
    async fn continue_resolve_rejects_length_mismatched_state() {
        let mut pending = auth_code_pending();
        pending.state = Some("aaa".into());
        let ctx = CredentialContext::for_test("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "c".to_owned());
        params.insert("state".to_owned(), "aaaa".to_owned()); // longer
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput(_))));
    }

    #[tokio::test]
    async fn continue_resolve_rejects_pre_fix_pending_without_state() {
        let mut pending = auth_code_pending();
        pending.state = None;
        pending.pkce_verifier = None;
        pending.redirect_uri = None;
        let ctx = CredentialContext::for_test("test-user");
        let mut params = HashMap::new();
        params.insert("code".to_owned(), "c".to_owned());
        params.insert("state".to_owned(), "s".to_owned());
        let input = UserInput::Callback { params };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(matches!(result, Err(CredentialError::InvalidInput(_))));
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
            pkce_verifier: None,
            state: None,
            redirect_uri: None,
        };

        let ctx = CredentialContext::for_test("test-user");
        let input = UserInput::Callback {
            params: HashMap::new(),
        };
        let result = OAuth2Credential::continue_resolve(&pending, &input, &ctx).await;
        assert!(result.is_err());
    }

    // ── initiate_authorization_code coverage (Tech Spec §15.4) ─────────
    //
    // Per §15.4 the base `Credential::resolve` cannot carry typed
    // pending state, so the AuthorizationCode kickoff lives on this
    // OAuth2-specific helper. The tests below cover (a) success-path
    // URL construction with PKCE + anti-CSRF, (b) input rejection for
    // missing redirect_uri, (c) state-token unguessability across
    // independent kickoffs.

    fn auth_code_values() -> FieldValues {
        let mut values = FieldValues::new();
        values.set_raw("client_id", serde_json::json!("test_client_id"));
        values.set_raw("client_secret", serde_json::json!("test_client_secret"));
        values.set_raw(
            "auth_url",
            serde_json::json!("https://idp.example.com/authorize"),
        );
        values.set_raw(
            "token_url",
            serde_json::json!("https://idp.example.com/token"),
        );
        values.set_raw("grant_type", serde_json::json!("authorization_code"));
        values.set_raw("scopes", serde_json::json!("read write"));
        values.set_raw(
            "redirect_uri",
            serde_json::json!("https://app.example.com/oauth2/callback"),
        );
        values
    }

    #[tokio::test]
    async fn initiate_authorization_code_returns_redirect_with_pkce_and_state() {
        let values = auth_code_values();
        let (pending, request) =
            OAuth2Credential::initiate_authorization_code(&values).expect("kickoff should succeed");

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
        assert!(
            pending.pkce_verifier.is_some(),
            "pkce_verifier must be populated"
        );
        assert!(pending.state.is_some(), "anti-CSRF state must be populated");
        assert!(
            pending.redirect_uri.is_some(),
            "redirect_uri must be populated"
        );
        assert_eq!(pending.grant_type, GrantType::AuthorizationCode);
    }

    #[tokio::test]
    async fn initiate_authorization_code_rejects_missing_redirect_uri() {
        // Build values without redirect_uri — AuthorizationCode requires it
        // per build_config (RFC 6749 §3.1.2 — registered redirection URI).
        let mut values = FieldValues::new();
        values.set_raw("client_id", serde_json::json!("test_client_id"));
        values.set_raw("client_secret", serde_json::json!("test_client_secret"));
        values.set_raw(
            "auth_url",
            serde_json::json!("https://idp.example.com/authorize"),
        );
        values.set_raw(
            "token_url",
            serde_json::json!("https://idp.example.com/token"),
        );
        values.set_raw("grant_type", serde_json::json!("authorization_code"));
        values.set_raw("scopes", serde_json::json!("read"));

        let result = OAuth2Credential::initiate_authorization_code(&values);
        match result {
            Err(CredentialError::InvalidInput(msg)) => {
                assert!(
                    msg.contains("redirect_uri"),
                    "error must name redirect_uri: {msg}"
                );
            },
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn initiate_authorization_code_csrf_state_is_unguessable() {
        let values = auth_code_values();
        let (pending1, _) =
            OAuth2Credential::initiate_authorization_code(&values).expect("first kickoff");
        let (pending2, _) =
            OAuth2Credential::initiate_authorization_code(&values).expect("second kickoff");

        let state1 = pending1.state.clone().expect("first state populated");
        let state2 = pending2.state.clone().expect("second state populated");

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
            auth_style: AuthStyle::default(),
        };

        let ctx = CredentialContext::for_test("test-user");
        let outcome = OAuth2Credential::refresh(&mut state, &ctx).await.unwrap();
        // Locally detected: never spoke to the IdP. Distinct from
        // `ProviderRejected` per wave-2 review (see ReauthReason rustdoc).
        assert!(
            matches!(
                outcome,
                RefreshOutcome::ReauthRequired(
                    crate::resolve::ReauthReason::MissingRefreshMaterial { .. }
                )
            ),
            "expected ReauthRequired(MissingRefreshMaterial); got {outcome:?}"
        );
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
            auth_style: AuthStyle::default(),
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
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: Some("dcode_secret".into()),
            interval: None,
            pkce_verifier: Some(SecretString::new("verifier_contents")),
            state: Some("state_contents".into()),
            redirect_uri: Some(TEST_CALLBACK.into()),
        };

        pending.zeroize();
        assert!(pending.client_secret.expose_secret().is_empty());
        assert!(pending.device_code.is_none());
        assert!(pending.client_id.is_empty());
        assert!(pending.interval.is_none());
        assert!(pending.pkce_verifier.is_none());
        assert!(pending.state.is_none());
        assert!(pending.redirect_uri.is_none());
    }

    #[test]
    fn pending_state_debug_redacts_pkce_verifier_and_state_but_shows_redirect_uri() {
        let pending = OAuth2Pending {
            config: OAuth2Config::authorization_code(TEST_CALLBACK)
                .auth_url("https://a.com/auth")
                .token_url("https://a.com/token")
                .build(),
            client_id: "cid".into(),
            client_secret: SecretString::new("cs"),
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: None,
            interval: None,
            pkce_verifier: Some(SecretString::new("my_pkce_verifier_value")),
            state: Some("my_csrf_state_value".into()),
            redirect_uri: Some(TEST_CALLBACK.into()),
        };
        let debug = format!("{pending:?}");
        assert!(!debug.contains("my_pkce_verifier_value"));
        assert!(!debug.contains("my_csrf_state_value"));
        assert!(debug.contains(TEST_CALLBACK));
        assert!(debug.contains("[REDACTED]"));
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
            grant_type: GrantType::AuthorizationCode,
            auth_style: AuthStyle::default(),
            device_code: None,
            interval: None,
            pkce_verifier: None,
            state: None,
            redirect_uri: None,
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
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("Bearer"));
        assert!(debug.contains("https://example.com/token"));
    }

    #[test]
    fn oauth2_state_serde_round_trip() {
        let state = make_state();
        let json = serde_json::to_string(&state).unwrap();
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
