//! Operator-supplied OAuth identity-provider configuration (Plane A).
//!
//! Per ADR-0085 D-1 (recon-4 reshaped), operator IdP-client credentials
//! live in `ApiConfig::auth.oauth.providers` as infrastructure config
//! — NOT in `CredentialService`. This matches the `SmtpEmailConfig`
//! precedent (operator infra creds stored in env-bound `ApiConfig`,
//! not user credentials).
//!
//! ## Endpoints shape (recon-4 ADOPT (b))
//!
//! Two arms via a tagged union:
//!
//! - [`OAuthEndpoints::Oidc`] — endpoints discovered from
//!   `.well-known/openid-configuration` at runtime (D-15). Scopes
//!   hardcoded `"openid email profile"`.
//! - [`OAuthEndpoints::Manual`] — explicit `authorize_url` /
//!   `token_url` / `userinfo_url` (+ optional `verified_emails_url`
//!   for providers like GitHub whose userinfo lacks `email_verified`,
//!   per ADR-0085 D-5 wave-6 addition); operator-supplied `scopes`.
//!
//! ## `redirect_uri` (recon-4 ADOPT (a))
//!
//! NOT a configuration field. Auto-derived at runtime as
//! `format!("{base}/auth/oauth/{provider}/callback", base = api_config.public_url)`.
//! Operators that need multiple callback URIs deploy multiple Nebula
//! instances. Matches n8n's `{instanceBaseUrl}/rest/sso/oidc/callback`
//! pattern.
//!
//! ## Scopes (recon-4 ADOPT (c))
//!
//! Hardcoded `"openid email profile"` for OIDC providers. Per-provider
//! `scopes` only for [`OAuthEndpoints::Manual`] (where the provider is
//! OAuth2-only and OIDC scope-claim semantics do not apply).

use std::collections::HashMap;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::domain::auth::backend::oauth::OAuthProvider;

/// OIDC scopes are hardcoded per ADR-0085 D-5 recon-4 ADOPT (c). Per-provider
/// scope customization belongs to the operator config only for the
/// [`OAuthEndpoints::Manual`] arm (OAuth2-only providers like GitHub).
pub const OIDC_HARDCODED_SCOPES: &[&str] = &["openid", "email", "profile"];

/// Map from `OAuthProvider` variant to per-provider config.
///
/// Defaults to empty (no OAuth providers declared); when non-empty the
/// composition root validates each entry at boot per ADR-0085
/// REQ-compose-001.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OAuthProvidersConfig {
    /// Declared providers, keyed by `OAuthProvider` enum value.
    ///
    /// Env discovery: a provider is "declared" iff
    /// `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID` is set in the environment.
    /// The 1.0 enum has three variants (Google / Microsoft / GitHub) —
    /// extending the enum is a 1.1 follow-up per ADR-0085 D-5.
    #[serde(default)]
    pub providers: HashMap<OAuthProvider, OAuthProviderConfig>,

    /// Test/dev-only: relax `validate_oauth_authorize_url` to accept
    /// `http://localhost(:port)?` for the **browser-fetched** authorize
    /// URL. Defaults to `false`. Per ADR-0085 D-9-WAVE6 the flag is
    /// scope-narrowed to authorize URLs only — server-side fetches
    /// (token / userinfo / verified_emails / jwks / discovery) keep the
    /// strict `validate_oauth_outbound_url` policy regardless.
    ///
    /// Env var: `API_AUTH_OAUTH_ALLOW_INSECURE_LOCALHOST`
    /// (`true` / `false`; default `false`). Production builds (release
    /// profile, `not(debug_assertions)`) reject the relaxation even when
    /// the flag is set — see `crates/api/src/lib.rs` release guard
    /// (PR-2 T2.12).
    #[serde(default)]
    pub oauth_allow_insecure_localhost: bool,
}

/// Per-provider OAuth identity-provider config (Plane A).
///
/// Constructed from env vars `API_AUTH_OAUTH_<PROVIDER>_*` (CLIENT_ID,
/// CLIENT_SECRET, DISCOVERY_URL or AUTHORIZE_URL / TOKEN_URL /
/// USERINFO_URL / VERIFIED_EMAILS_URL / JWKS_URL / SCOPES). The
/// `endpoints` arm is inferred from which env vars are set:
/// `DISCOVERY_URL` present → [`OAuthEndpoints::Oidc`]; otherwise →
/// [`OAuthEndpoints::Manual`].
///
/// `client_id` and `client_secret` are `SecretString` so `Debug`
/// redacts them and the buffers zeroize on drop, matching
/// `SmtpEmailConfig.password`.
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuthProviderConfig {
    /// OAuth client_id issued by the IdP. Env: `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID`.
    ///
    /// `#[serde(skip)]` because the value is only ever populated from
    /// the environment (matches `SmtpEmailConfig::password` precedent);
    /// serializing a credential to a JSON snapshot would silently leak
    /// it through `tracing` / `problem-details` paths.
    #[serde(skip)]
    pub client_id: SecretString,
    /// OAuth client_secret issued by the IdP. Env: `API_AUTH_OAUTH_<PROVIDER>_CLIENT_SECRET`.
    /// `#[serde(skip)]` for the same reason as `client_id`.
    #[serde(skip)]
    pub client_secret: SecretString,
    /// Resolved endpoints (tagged union per ADR-0085 D-5).
    pub endpoints: OAuthEndpoints,
}

impl Default for OAuthProviderConfig {
    /// Empty placeholder. `serde(skip)` on the secrets means deserialize
    /// goes through `Default::default()` for those fields; the
    /// `validate_at_load` step (PR-2 T2.8) rejects the empty values at
    /// boot per REQ-compose-001 Invariant 1, so the empty default never
    /// reaches a running OAuth flow.
    fn default() -> Self {
        Self {
            client_id: SecretString::new(String::new().into_boxed_str()),
            client_secret: SecretString::new(String::new().into_boxed_str()),
            endpoints: OAuthEndpoints::Oidc {
                discovery_url: String::new(),
            },
        }
    }
}

impl std::fmt::Debug for OAuthProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual Debug to avoid leaking the `SecretString` body even
        // through derived fmt — mirrors `SmtpEmailConfig::Debug`.
        f.debug_struct("OAuthProviderConfig")
            .field("client_id", &"[redacted]")
            .field("client_secret", &"[redacted]")
            .field("endpoints", &self.endpoints)
            .finish()
    }
}

/// Tagged union of OAuth endpoint shapes per ADR-0085 D-5 recon-4.
///
/// `kind = "oidc"` for providers exposing `.well-known/openid-configuration`
/// (Google / Microsoft / Auth0 / Okta — though Auth0/Okta require an
/// `OAuthProvider` enum extension scheduled for 1.1).
///
/// `kind = "manual"` for OAuth2-only providers (GitHub) or
/// operator-customized OIDC where the operator wants to pin endpoints
/// against a staging mirror.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OAuthEndpoints {
    /// OIDC provider — runtime endpoint discovery from
    /// `.well-known/openid-configuration` (D-15). Scopes hardcoded
    /// `"openid email profile"` per ADR-0085 D-5 recon-4 ADOPT (c).
    Oidc {
        /// Fully-qualified URL of the OIDC discovery document. Validated
        /// at boot by the strict `validate_oauth_outbound_url` gate
        /// (HTTPS, no localhost / private / loopback / multicast IPs)
        /// per ADR-0085 D-9-WAVE6.
        discovery_url: String,
    },
    /// OAuth2-only or operator-customized provider — explicit endpoint
    /// URLs.
    Manual {
        /// OAuth authorize endpoint (browser-fetched). Validated by the
        /// **flag-aware** `validate_oauth_authorize_url` gate
        /// (`oauth_allow_insecure_localhost` flag relaxes `http://localhost`
        /// when set AND `!cfg!(debug_assertions)`).
        authorize_url: String,
        /// OAuth token endpoint (server-fetched). Strict gate.
        token_url: String,
        /// OAuth userinfo endpoint (server-fetched, with `Authorization:
        /// Bearer <access_token>`). Strict gate. The response is the
        /// authoritative source for `(email, sub)` per ADR-0085 D-16
        /// (id_token JWKS validation deferred to 1.1).
        userinfo_url: String,
        /// Optional second userinfo endpoint for providers whose primary
        /// userinfo response lacks `email_verified` (GitHub's
        /// `/user/emails`). Strict gate. Per ADR-0085 D-5 wave-6, PR-4
        /// fetches this AFTER `userinfo_url` and picks the entry where
        /// `primary == true AND verified == true`. `None` means the
        /// primary userinfo response includes `email_verified` inline
        /// (the OIDC norm).
        #[serde(default)]
        verified_emails_url: Option<String>,
        /// Optional JWKS URL. Accepted for forward compat but ignored in
        /// 1.0 per ADR-0085 D-16 (id_token signature validation deferred
        /// to 1.1). Strict gate when present.
        #[serde(default)]
        jwks_url: Option<String>,
        /// Per-provider scopes (non-empty). OAuth2-only providers (e.g.
        /// GitHub needs `["user:email"]` for `/user/emails` access).
        scopes: Vec<String>,
    },
}

impl OAuthEndpoints {
    /// Strict-gate helper: collect every server-side URL that
    /// `validate_oauth_outbound_url` must approve at boot. Per ADR-0085
    /// D-9-WAVE6 the strict gate covers token / userinfo /
    /// verified_emails / jwks / discovery; the authorize URL goes
    /// through the flag-aware gate via [`Self::authorize_url`].
    #[must_use]
    pub fn server_side_urls(&self) -> Vec<&str> {
        match self {
            Self::Oidc { discovery_url } => vec![discovery_url.as_str()],
            Self::Manual {
                token_url,
                userinfo_url,
                verified_emails_url,
                jwks_url,
                ..
            } => {
                let mut urls = vec![token_url.as_str(), userinfo_url.as_str()];
                if let Some(url) = verified_emails_url.as_deref() {
                    urls.push(url);
                }
                if let Some(url) = jwks_url.as_deref() {
                    urls.push(url);
                }
                urls
            },
        }
    }

    /// Flag-aware-gate helper: the operator-supplied authorize URL when
    /// the provider is `Manual` (the OIDC arm's authorize URL is
    /// discovered at runtime and validated then per D-15-WAVE6).
    #[must_use]
    pub fn authorize_url(&self) -> Option<&str> {
        match self {
            Self::Oidc { .. } => None,
            Self::Manual { authorize_url, .. } => Some(authorize_url.as_str()),
        }
    }

    /// Scope resolution per ADR-0085 D-5 recon-4 ADOPT (c): hardcoded
    /// `"openid email profile"` for OIDC; operator-supplied for Manual.
    #[must_use]
    pub fn scopes(&self) -> Vec<String> {
        match self {
            Self::Oidc { .. } => OIDC_HARDCODED_SCOPES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            Self::Manual { scopes, .. } => scopes.clone(),
        }
    }
}
