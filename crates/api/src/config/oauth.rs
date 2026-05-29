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
//! `format!("{base}/api/v1/auth/oauth/{provider}/callback", base = api_config.public_url)`
//! (the Plane-A router is nested under `/api/v1/` per
//! `crates/api/src/domain/mod.rs`).
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

use super::errors::ApiConfigError;
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
    /// URL ONLY in dev builds. Defaults to `false`. Per ADR-0085
    /// D-9-WAVE6 the flag is scope-narrowed to authorize URLs only —
    /// server-side fetches
    /// (token / userinfo / verified_emails / jwks / discovery) keep the
    /// strict `validate_oauth_outbound_url` policy regardless.
    ///
    /// Env var: `API_AUTH_OAUTH_ALLOW_INSECURE_LOCALHOST`
    /// (`true` / `false`; default `false`). Production builds (release
    /// profile — `cfg!(debug_assertions) == false`) reject the
    /// relaxation even when the flag is set; the flag only takes
    /// effect in dev builds where `cfg!(debug_assertions) == true`.
    /// See `crates/api/src/lib.rs` release guard (PR-2 T2.12).
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

/// Failure reason from [`OAuthProvidersConfig::validate_at_load`].
///
/// Stable keyword strings the operator can grep for in docs. Mapped to
/// `TransportInitError::OAuthProviderConfigInvalid { provider, reason }`
/// in the composition root.
#[derive(Debug, PartialEq, Eq)]
pub struct OAuthConfigValidationError {
    /// Provider name (snake_case OAuthProvider enum variant).
    pub provider: String,
    /// Stable reason keyword.
    pub reason: &'static str,
}

/// All `OAuthProvider` enum variants supported in 1.0. Used by
/// [`OAuthProvidersConfig::from_env`] to drive the per-provider env
/// scan. Adding a variant to the enum + this slice is the entire
/// surface for a 1.1 enum-extension follow-up per ADR-0085 D-5.
///
/// `pub(crate)` so the rustdoc intra-doc link in `from_env`'s public
/// doc-block resolves (private items cannot be linked from public
/// doc); production callers should not reach for this directly — use
/// `OAuthProvider` enum iteration or [`OAuthProvidersConfig::from_env`]
/// instead.
pub(crate) const KNOWN_PROVIDERS: &[OAuthProvider] = &[
    OAuthProvider::Google,
    OAuthProvider::Microsoft,
    OAuthProvider::GitHub,
];

impl OAuthProvidersConfig {
    /// Discover declared OAuth providers from environment variables.
    ///
    /// For each `OAuthProvider` enum variant the loader looks for the
    /// `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID` env var as the
    /// declaration sentinel. If present (non-empty after trim), the
    /// provider is added to the map with the rest of the per-provider
    /// env vars consumed:
    ///
    /// - `API_AUTH_OAUTH_<PROVIDER>_CLIENT_ID` — required.
    /// - `API_AUTH_OAUTH_<PROVIDER>_CLIENT_SECRET` — required.
    /// - **Discriminator**: `DISCOVERY_URL` set →
    ///   [`OAuthEndpoints::Oidc`]; otherwise [`OAuthEndpoints::Manual`].
    /// - Oidc: `DISCOVERY_URL` required.
    /// - Manual: `AUTHORIZE_URL` / `TOKEN_URL` / `USERINFO_URL`
    ///   required; `VERIFIED_EMAILS_URL` / `JWKS_URL` optional;
    ///   `SCOPES` required (whitespace- or comma-separated).
    ///
    /// Plus the global flag
    /// `API_AUTH_OAUTH_ALLOW_INSECURE_LOCALHOST` (`true`/`false`,
    /// default `false`).
    ///
    /// Provider keys not in the KNOWN_PROVIDERS slice (e.g. typos like
    /// `API_AUTH_OAUTH_GOOOGLE_CLIENT_ID`) are silently ignored at
    /// the env-loader level — the OAuthProvider enum has no parser for
    /// them so a typo simply never matches any iteration. Boot-time
    /// validation in [`Self::validate_at_load`] catches
    /// half-populated configs (CLIENT_ID set but neither discovery nor
    /// authorize URL).
    ///
    /// # Errors
    ///
    /// Returns [`ApiConfigError::ParseEnum`] when a provider has
    /// `CLIENT_ID` set but neither `DISCOVERY_URL` nor `AUTHORIZE_URL`
    /// (ambiguous shape — cannot pick Oidc vs Manual).
    pub fn from_env() -> Result<Self, ApiConfigError> {
        let allow_localhost =
            super::env::parse_bool_env("AUTH_OAUTH_ALLOW_INSECURE_LOCALHOST", false)?;
        let mut providers = HashMap::new();

        for provider in KNOWN_PROVIDERS {
            let upper = provider.as_str().to_ascii_uppercase();
            let prefix = format!("API_AUTH_OAUTH_{upper}");
            let client_id_var = format!("{prefix}_CLIENT_ID");
            let raw_client_id = std::env::var(&client_id_var).unwrap_or_default();
            if raw_client_id.trim().is_empty() {
                // Provider not declared. Move on.
                continue;
            }
            let raw_client_secret =
                std::env::var(format!("{prefix}_CLIENT_SECRET")).unwrap_or_default();

            let discovery_url = std::env::var(format!("{prefix}_DISCOVERY_URL"))
                .ok()
                .filter(|s| !s.trim().is_empty());
            let authorize_url = std::env::var(format!("{prefix}_AUTHORIZE_URL"))
                .ok()
                .filter(|s| !s.trim().is_empty());

            let endpoints = match (discovery_url, authorize_url) {
                (Some(url), _) => OAuthEndpoints::Oidc { discovery_url: url },
                (None, Some(authorize_url)) => OAuthEndpoints::Manual {
                    authorize_url,
                    token_url: std::env::var(format!("{prefix}_TOKEN_URL")).unwrap_or_default(),
                    userinfo_url: std::env::var(format!("{prefix}_USERINFO_URL"))
                        .unwrap_or_default(),
                    verified_emails_url: std::env::var(format!("{prefix}_VERIFIED_EMAILS_URL"))
                        .ok()
                        .filter(|s| !s.trim().is_empty()),
                    jwks_url: std::env::var(format!("{prefix}_JWKS_URL"))
                        .ok()
                        .filter(|s| !s.trim().is_empty()),
                    scopes: nebula_env::list(&format!("{prefix}_SCOPES")),
                },
                (None, None) => {
                    return Err(ApiConfigError::ParseEnum {
                        var: "AUTH_OAUTH_*_DISCOVERY_URL_or_AUTHORIZE_URL",
                        raw: format!(
                            "provider `{}` has {prefix}_CLIENT_ID set but neither {prefix}_DISCOVERY_URL nor {prefix}_AUTHORIZE_URL; declare exactly one to pick the OAuthEndpoints arm",
                            provider.as_str()
                        ),
                    });
                },
            };

            providers.insert(
                *provider,
                OAuthProviderConfig {
                    client_id: SecretString::new(raw_client_id.into_boxed_str()),
                    client_secret: SecretString::new(raw_client_secret.into_boxed_str()),
                    endpoints,
                },
            );
        }

        Ok(Self {
            providers,
            oauth_allow_insecure_localhost: allow_localhost,
        })
    }

    /// Validate every declared provider at boot per ADR-0085
    /// REQ-compose-001 Invariant 1 (recon-4 + wave-6/-7 hardening).
    ///
    /// Returns the FIRST validation failure (boot is fail-fast — the
    /// operator gets one error at a time and fixes them in order).
    /// When the config is empty (default), returns `Ok(())` without
    /// any work — OAuth is opt-in.
    ///
    /// `public_url` is the value from `ApiConfig::public_url` (handed
    /// in so this method has no implicit dependency on the wider
    /// `ApiConfig` shape). When the providers map is non-empty,
    /// `public_url` MUST be non-empty AND absolute (with scheme);
    /// the boot-derived `redirect_uri` per ADR-0085 D-3 depends on it.
    ///
    /// `in_release_build` is `cfg!(not(debug_assertions))` at the call
    /// site, threaded explicitly so this function stays unit-testable
    /// without recompiling under release profile.
    ///
    /// # Errors
    ///
    /// Returns [`OAuthConfigValidationError`] on the first invalid
    /// field. See the type's `reason` doc for the stable keyword set.
    pub fn validate_at_load(
        &self,
        public_url: &str,
        in_release_build: bool,
    ) -> Result<(), OAuthConfigValidationError> {
        if self.providers.is_empty() {
            return Ok(());
        }

        // `public_url` is required for the auto-derived `redirect_uri`
        // formula per D-3 recon-4. Empty or scheme-less values are a
        // boot-time error because every OAuth flow would build a
        // broken redirect URL otherwise.
        //
        // Wave-1 review (Codex / Copilot / CodeRabbit): parse via
        // `url::Url` rather than a prefix check so malformed values like
        // `"https://"` (no host) AND opaque schemes like `"data:..."`
        // are rejected.
        //
        // Wave-2 review (Codex P2): do NOT silently trim. The value
        // validated here is also what `AppState.public_url` carries at
        // runtime; trimming on validate but storing the un-trimmed
        // value would diverge, and a leading space would break the
        // `redirect_uri` round-trip on `complete_oauth`. Reject
        // whitespace explicitly so the operator fixes the env var.
        let parsed = url::Url::parse(public_url);
        let public_url_ok = public_url == public_url.trim()
            && match parsed {
                Ok(ref u) => (u.scheme() == "http" || u.scheme() == "https") && u.has_host(),
                Err(_) => false,
            };
        if !public_url_ok {
            // Use first provider name for the error — the issue is
            // global but reporting against a concrete provider helps
            // the operator find their env-var set.
            let provider = self
                .providers
                .keys()
                .next()
                .map(|p| p.as_str().to_owned())
                .unwrap_or_else(|| "<unknown>".to_owned());
            return Err(OAuthConfigValidationError {
                provider,
                reason: "public_url_required",
            });
        }

        for (provider, cfg) in &self.providers {
            let p = provider.as_str().to_owned();
            cfg.validate(&p, self.oauth_allow_insecure_localhost, in_release_build)?;
        }
        Ok(())
    }
}

impl OAuthProviderConfig {
    /// Validate a single provider config at boot. Called by
    /// [`OAuthProvidersConfig::validate_at_load`].
    fn validate(
        &self,
        provider: &str,
        oauth_allow_insecure_localhost: bool,
        in_release_build: bool,
    ) -> Result<(), OAuthConfigValidationError> {
        use secrecy::ExposeSecret;

        if self.client_id.expose_secret().is_empty() {
            return Err(OAuthConfigValidationError {
                provider: provider.to_owned(),
                reason: "client_id_required",
            });
        }
        if self.client_secret.expose_secret().is_empty() {
            return Err(OAuthConfigValidationError {
                provider: provider.to_owned(),
                reason: "client_secret_required",
            });
        }
        self.endpoints
            .validate(provider, oauth_allow_insecure_localhost, in_release_build)?;
        Ok(())
    }
}

impl OAuthEndpoints {
    /// Validate per-endpoint per its threat model per ADR-0085
    /// D-9-WAVE6 + D-15-WAVE6 + F.2 wave-7:
    /// - Strict gate (`validate_oauth_outbound_url`) for server-side
    ///   fetches: token / userinfo / verified_emails / jwks / discovery.
    /// - Flag-aware gate (`validate_oauth_authorize_url`) for the
    ///   browser-fetched `Manual.authorize_url`.
    fn validate(
        &self,
        provider: &str,
        oauth_allow_insecure_localhost: bool,
        in_release_build: bool,
    ) -> Result<(), OAuthConfigValidationError> {
        use crate::transport::oauth::flow::{
            validate_oauth_authorize_url, validate_oauth_outbound_url,
        };

        // Strict gate for ALL server-side URLs.
        for url in self.server_side_urls() {
            validate_oauth_outbound_url(url).map_err(|_| OAuthConfigValidationError {
                provider: provider.to_owned(),
                reason: "endpoint_url_must_be_https",
            })?;
        }

        // Flag-aware gate for the browser-fetched Manual.authorize_url
        // (Oidc.authorize_url is discovered at runtime per D-15-WAVE6
        // and validated then; not represented in operator config).
        if let Some(authorize_url) = self.authorize_url() {
            validate_oauth_authorize_url(
                authorize_url,
                oauth_allow_insecure_localhost,
                in_release_build,
            )
            .map_err(|_| OAuthConfigValidationError {
                provider: provider.to_owned(),
                reason: "authorize_url_invalid_or_localhost_in_release",
            })?;
        }

        // Manual.scopes must be non-empty.
        if let Self::Manual { scopes, .. } = self
            && scopes.is_empty()
        {
            return Err(OAuthConfigValidationError {
                provider: provider.to_owned(),
                reason: "manual_scopes_required",
            });
        }

        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::auth::backend::oauth::OAuthProvider;

    fn mk_oidc(discovery_url: &str) -> OAuthProviderConfig {
        OAuthProviderConfig {
            client_id: SecretString::new("client".into()),
            client_secret: SecretString::new("secret".into()),
            endpoints: OAuthEndpoints::Oidc {
                discovery_url: discovery_url.to_owned(),
            },
        }
    }

    fn mk_manual(
        authorize_url: &str,
        token_url: &str,
        userinfo_url: &str,
        scopes: Vec<String>,
    ) -> OAuthProviderConfig {
        OAuthProviderConfig {
            client_id: SecretString::new("client".into()),
            client_secret: SecretString::new("secret".into()),
            endpoints: OAuthEndpoints::Manual {
                authorize_url: authorize_url.to_owned(),
                token_url: token_url.to_owned(),
                userinfo_url: userinfo_url.to_owned(),
                verified_emails_url: None,
                jwks_url: None,
                scopes,
            },
        }
    }

    fn cfg_with(provider: OAuthProvider, entry: OAuthProviderConfig) -> OAuthProvidersConfig {
        let mut cfg = OAuthProvidersConfig::default();
        cfg.providers.insert(provider, entry);
        cfg
    }

    /// T2.3 RED-then-GREEN: OIDC providers require an HTTPS discovery_url
    /// per ADR-0085 REQ-compose-001 Invariant 1.
    #[test]
    fn oauth_provider_config_validates_oidc_requires_https_discovery_url() {
        let cfg = cfg_with(
            OAuthProvider::Google,
            mk_oidc("http://accounts.google.com/.well-known/openid-configuration"),
        );
        let err = cfg
            .validate_at_load("https://app.example.com", false)
            .expect_err("HTTP discovery_url must be rejected by the strict gate");
        assert_eq!(err.provider, "google");
        assert_eq!(err.reason, "endpoint_url_must_be_https");
    }

    /// T2.4 RED-then-GREEN: Manual providers require non-empty scopes.
    #[test]
    fn oauth_provider_config_validates_manual_requires_non_empty_scopes() {
        let cfg = cfg_with(
            OAuthProvider::GitHub,
            mk_manual(
                "https://github.com/login/oauth/authorize",
                "https://github.com/login/oauth/access_token",
                "https://api.github.com/user",
                vec![],
            ),
        );
        let err = cfg
            .validate_at_load("https://app.example.com", false)
            .expect_err("Manual provider with empty scopes must be rejected");
        assert_eq!(err.provider, "github");
        assert_eq!(err.reason, "manual_scopes_required");
    }

    /// T2.5(a) RED-then-GREEN: server-side URLs go through STRICT
    /// `validate_oauth_outbound_url` regardless of the
    /// `oauth_allow_insecure_localhost` flag.
    #[test]
    fn oauth_provider_config_rejects_http_endpoint_for_all_server_side_urls() {
        let cfg = cfg_with(
            OAuthProvider::GitHub,
            mk_manual(
                "https://github.com/login/oauth/authorize",
                "http://github.com/login/oauth/access_token", // HTTP — must be rejected even with flag ON
                "https://api.github.com/user",
                vec!["user:email".to_owned()],
            ),
        );
        let mut providers = cfg;
        providers.oauth_allow_insecure_localhost = true; // flag has no effect on token_url
        let err = providers
            .validate_at_load("https://app.example.com", false)
            .expect_err("HTTP token_url rejected by strict gate even with localhost flag set");
        assert_eq!(err.reason, "endpoint_url_must_be_https");
    }

    /// T2.5(b) RED-then-GREEN: `Manual.authorize_url` uses the FLAG-AWARE
    /// `validate_oauth_authorize_url` which accepts `http://localhost`
    /// when the flag is set AND the binary is not a release build.
    #[test]
    fn oauth_provider_config_authorize_url_uses_flag_aware_validator() {
        let cfg = cfg_with(
            OAuthProvider::GitHub,
            mk_manual(
                "http://localhost:8088/authorize", // would fail strict gate
                "https://github.com/login/oauth/access_token",
                "https://api.github.com/user",
                vec!["user:email".to_owned()],
            ),
        );
        let mut providers = cfg;
        providers.oauth_allow_insecure_localhost = true;
        // in_release_build = false simulates `cfg!(debug_assertions)`.
        providers
            .validate_at_load("https://app.example.com", false)
            .expect("flag-aware gate must accept localhost authorize_url in dev mode");
    }

    /// T2.5(c) RED-then-GREEN: the localhost flag has NO effect in
    /// release builds — `validate_oauth_authorize_url` rejects.
    #[test]
    fn oauth_authorize_url_strict_in_release_build() {
        let cfg = cfg_with(
            OAuthProvider::GitHub,
            mk_manual(
                "http://localhost:8088/authorize",
                "https://github.com/login/oauth/access_token",
                "https://api.github.com/user",
                vec!["user:email".to_owned()],
            ),
        );
        let mut providers = cfg;
        providers.oauth_allow_insecure_localhost = true;
        let err = providers
            .validate_at_load("https://app.example.com", true) // release build
            .expect_err("flag must have no effect in release builds");
        assert_eq!(err.reason, "authorize_url_invalid_or_localhost_in_release");
    }

    /// T2.6 RED-then-GREEN: compose-root MUST fail closed when OAuth is
    /// declared but `ApiConfig::public_url` is empty.
    #[test]
    fn validate_at_load_fails_closed_when_public_url_unset_with_oauth_declared() {
        let cfg = cfg_with(
            OAuthProvider::Google,
            mk_oidc("https://accounts.google.com/.well-known/openid-configuration"),
        );
        let err = cfg
            .validate_at_load("", false)
            .expect_err("empty public_url with OAuth declared must fail closed");
        assert_eq!(err.reason, "public_url_required");
    }

    /// Sanity: empty providers map is a no-op (OAuth is opt-in).
    #[test]
    fn validate_at_load_noop_when_providers_empty() {
        let cfg = OAuthProvidersConfig::default();
        cfg.validate_at_load("", false)
            .expect("empty providers is opt-in, no validation needed");
    }

    /// REQ-oauth-001 Invariant 1 coverage for the OIDC scope hardcoding
    /// (recon-4 ADOPT (c)): OIDC providers ALWAYS expose
    /// `["openid","email","profile"]` regardless of any operator-supplied
    /// scopes (Oidc variant has no scopes field by construction).
    #[test]
    fn oidc_endpoints_always_emit_hardcoded_scopes() {
        let endpoints = OAuthEndpoints::Oidc {
            discovery_url: "https://accounts.google.com/.well-known/openid-configuration"
                .to_owned(),
        };
        assert_eq!(endpoints.scopes(), vec!["openid", "email", "profile"]);
    }

    /// T2.13 TRIANGULATE: `validate_at_load` rejects a `public_url`
    /// that has a scheme but no host (e.g. `"https://"`). Closes the
    /// Url-parse path that the wave-1 review flagged as a major gap.
    #[test]
    fn validate_at_load_rejects_public_url_with_no_host() {
        let cfg = cfg_with(
            OAuthProvider::Google,
            mk_oidc("https://accounts.google.com/.well-known/openid-configuration"),
        );
        let err = cfg
            .validate_at_load("https://", false)
            .expect_err("scheme without host must be rejected");
        assert_eq!(err.reason, "public_url_required");
    }

    /// T2.13 TRIANGULATE wave-2 (Codex P2): whitespace-padded
    /// `public_url` rejected explicitly — do NOT silently trim because
    /// the validated value and the value stored in `AppState` must
    /// match exactly, or the `redirect_uri` round-trip on
    /// `complete_oauth` will diverge.
    #[test]
    fn validate_at_load_rejects_whitespace_padded_public_url() {
        let cfg = cfg_with(
            OAuthProvider::Google,
            mk_oidc("https://accounts.google.com/.well-known/openid-configuration"),
        );
        let err = cfg
            .validate_at_load(" https://app.example.com", false)
            .expect_err("leading whitespace must be rejected");
        assert_eq!(err.reason, "public_url_required");

        let err = cfg
            .validate_at_load("https://app.example.com\n", false)
            .expect_err("trailing whitespace / newline must be rejected");
        assert_eq!(err.reason, "public_url_required");
    }

    /// T2.13 TRIANGULATE: opaque-scheme `public_url` is rejected (the
    /// boot-time gate must NOT accept `data:` / `file:` / `javascript:`
    /// for the OAuth redirect base — those don't form a server URL).
    #[test]
    fn validate_at_load_rejects_public_url_with_opaque_scheme() {
        let cfg = cfg_with(
            OAuthProvider::Google,
            mk_oidc("https://accounts.google.com/.well-known/openid-configuration"),
        );
        let err = cfg
            .validate_at_load("javascript:alert(1)", false)
            .expect_err("opaque scheme rejected");
        assert_eq!(err.reason, "public_url_required");
    }

    /// T2.13 TRIANGULATE: the scopes env value (parsed via `nebula_env::list`)
    /// accepts both whitespace- and comma-separated forms (operator-friendly).
    #[test]
    fn parse_scopes_env_accepts_whitespace_and_commas() {
        let var = "API_AUTH_OAUTH_TEST_T213_SCOPES_PARSE";
        let mut env = nebula_env::testing::EnvGuard::acquire();
        env.set(var, "user:email read:user,write:repo  openid");
        let scopes = nebula_env::list(var);
        assert_eq!(
            scopes,
            vec![
                "user:email".to_owned(),
                "read:user".to_owned(),
                "write:repo".to_owned(),
                "openid".to_owned()
            ]
        );
    }

    /// Manual providers honor operator-supplied scopes verbatim.
    #[test]
    fn manual_endpoints_emit_operator_supplied_scopes() {
        let endpoints = OAuthEndpoints::Manual {
            authorize_url: "https://github.com/login/oauth/authorize".to_owned(),
            token_url: "https://github.com/login/oauth/access_token".to_owned(),
            userinfo_url: "https://api.github.com/user".to_owned(),
            verified_emails_url: Some("https://api.github.com/user/emails".to_owned()),
            jwks_url: None,
            scopes: vec!["user:email".to_owned(), "read:user".to_owned()],
        };
        assert_eq!(endpoints.scopes(), vec!["user:email", "read:user"]);
    }
}
