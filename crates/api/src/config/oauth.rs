//! Operator-supplied OAuth identity-provider configuration (Plane A).
//!
//! Per ADR-0085 D-1, operator IdP-client credentials live in
//! `ApiConfig::auth.oauth.providers` as infrastructure config, not in
//! `CredentialService`.
//!
//! Endpoint, scope, and token-authentication policy is deliberately absent
//! from operator configuration. Each admitted provider name selects one
//! runtime-owned, reviewed profile: canonical Google OIDC discovery or
//! canonical GitHub.com OAuth endpoints. Supporting another profile is an
//! application change, not an environment toggle.
//!
//! `redirect_uri` is also not a configuration field. The callback path is
//! appended with URL path-segment APIs to the validated external mount prefix
//! in `ApiConfig::public_url`.

use std::collections::HashMap;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::errors::ApiConfigError;
use crate::domain::auth::backend::OAuthProvider;

/// Map from an admitted provider to its operator-owned client credentials.
///
/// The map is empty by default, which disables OAuth. Provider names select
/// private runtime profiles; this type intentionally has no endpoint, scope,
/// token-authentication, or development-bypass fields.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthProvidersConfig {
    /// Declared providers, keyed by the closed [`OAuthProvider`] enum.
    ///
    /// Environment discovery treats a provider as declared when either its
    /// client-id or client-secret variable is present. Retaining incomplete
    /// declarations makes boot validation fail closed.
    #[serde(default)]
    pub providers: HashMap<OAuthProvider, OAuthProviderConfig>,
}

/// Operator-owned client credentials for one admitted provider.
///
/// Both fields are skipped by serde to prevent credential material from
/// entering snapshots or diagnostic serialization. Deserialization therefore
/// produces empty placeholders which boot validation rejects.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthProviderConfig {
    /// OAuth client identifier issued by the provider.
    #[serde(skip)]
    pub client_id: SecretString,
    /// OAuth client secret issued by the provider.
    #[serde(skip)]
    pub client_secret: SecretString,
}

impl Default for OAuthProviderConfig {
    fn default() -> Self {
        Self {
            client_id: SecretString::new(String::new().into_boxed_str()),
            client_secret: SecretString::new(String::new().into_boxed_str()),
        }
    }
}

impl std::fmt::Debug for OAuthProviderConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OAuthProviderConfig")
            .field("client_id", &"[redacted]")
            .field("client_secret", &"[redacted]")
            .finish()
    }
}

/// Failure reason from [`OAuthProvidersConfig::validate_at_load`].
///
/// Stable keyword strings are mapped to the composition root's secret-free
/// initialization error.
#[derive(Debug, PartialEq, Eq)]
pub struct OAuthConfigValidationError {
    /// Provider name in its public snake-case representation.
    pub provider: String,
    /// Stable reason keyword.
    pub reason: &'static str,
}

/// Providers with complete, reviewed runtime identity contracts.
pub(crate) const KNOWN_PROVIDERS: &[OAuthProvider] =
    &[OAuthProvider::Google, OAuthProvider::GitHub];

/// Parse the canonical public base URL used to derive OAuth callback URLs.
///
/// A root URL and a plain reverse-proxy base path are supported; credentials,
/// query, fragment, encoded, dot, and empty path segments are rejected.
pub(crate) fn parse_public_oauth_base_url(
    public_url: &str,
    in_release_build: bool,
) -> Result<url::Url, ()> {
    if public_url != public_url.trim()
        || public_url.contains('\\')
        || public_url.contains('%')
        || public_url.chars().any(char::is_control)
    {
        return Err(());
    }
    let mut url = url::Url::parse(public_url).map_err(|_| ())?;
    let raw_path = public_url
        .split_once("://")
        .and_then(|(_, authority_and_path)| {
            authority_and_path
                .find('/')
                .map(|at| &authority_and_path[at..])
        })
        .unwrap_or("")
        .split(['?', '#'])
        .next()
        .unwrap_or("");
    if raw_path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(());
    }
    let localhost = match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            host.eq_ignore_ascii_case("localhost")
                || host.to_ascii_lowercase().ends_with(".localhost")
        },
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    let scheme_ok = if in_release_build {
        url.scheme() == "https" && !localhost
    } else {
        url.scheme() == "https" || (url.scheme() == "http" && localhost)
    };
    let path = url.path();
    let path_ok = !path.contains("//")
        && path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .all(|segment| segment != "." && segment != "..");
    if scheme_ok
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && path_ok
    {
        if url.path().len() > 1 && url.path().ends_with('/') {
            let canonical_path = url.path().trim_end_matches('/').to_owned();
            url.set_path(&canonical_path);
        }
        Ok(url)
    } else {
        Err(())
    }
}

impl OAuthProvidersConfig {
    /// Discover the closed OAuth credential configuration from environment.
    ///
    /// The only accepted variables are
    /// `API_AUTH_OAUTH_{GOOGLE,GITHUB}_{CLIENT_ID,CLIENT_SECRET}`. Reserved
    /// provider-profile variables fail even when their values are empty, so a
    /// stale deployment declaration cannot silently become a no-op.
    ///
    /// # Errors
    ///
    /// Returns a fixed, secret-free error for parked, unknown, or legacy
    /// configuration declarations.
    pub fn from_env() -> Result<Self, ApiConfigError> {
        let oauth_vars = || {
            std::env::vars_os().filter_map(|(key, _)| {
                key.to_str()
                    .and_then(|key| key.strip_prefix("API_AUTH_OAUTH_"))
                    .map(str::to_owned)
            })
        };

        if oauth_vars().any(|suffix| suffix.starts_with("MICROSOFT_")) {
            return Err(ApiConfigError::OAuthProviderParked {
                provider: "microsoft",
            });
        }
        if oauth_vars()
            .any(|suffix| suffix.ends_with("_JWKS_URL") && suffix.len() > "_JWKS_URL".len())
        {
            return Err(ApiConfigError::OAuthConfigUnsupported {
                reason: "jwks_url_unsupported",
            });
        }
        if oauth_vars().any(|suffix| {
            if suffix == "ALLOW_INSECURE_LOCALHOST" {
                return false;
            }
            let provider = suffix
                .split_once('_')
                .map_or(suffix.as_str(), |(provider, _)| provider);
            !matches!(provider, "GOOGLE" | "GITHUB")
        }) {
            return Err(ApiConfigError::OAuthConfigUnsupported {
                reason: "provider_unknown",
            });
        }
        if oauth_vars().any(|suffix| {
            if suffix == "ALLOW_INSECURE_LOCALHOST" {
                return true;
            }
            let Some((provider, field)) = suffix.split_once('_') else {
                return true;
            };
            matches!(provider, "GOOGLE" | "GITHUB")
                && !matches!(field, "CLIENT_ID" | "CLIENT_SECRET")
        }) {
            return Err(ApiConfigError::OAuthConfigUnsupported {
                reason: "provider_profile_override_unsupported",
            });
        }

        let mut providers = HashMap::new();
        for provider in KNOWN_PROVIDERS {
            let upper = provider.as_str().to_ascii_uppercase();
            let prefix = format!("API_AUTH_OAUTH_{upper}");
            let client_id_var = format!("{prefix}_CLIENT_ID");
            let client_secret_var = format!("{prefix}_CLIENT_SECRET");
            if std::env::var_os(&client_id_var).is_none()
                && std::env::var_os(&client_secret_var).is_none()
            {
                continue;
            }
            providers.insert(
                *provider,
                OAuthProviderConfig {
                    client_id: SecretString::new(
                        std::env::var(client_id_var)
                            .unwrap_or_default()
                            .into_boxed_str(),
                    ),
                    client_secret: SecretString::new(
                        std::env::var(client_secret_var)
                            .unwrap_or_default()
                            .into_boxed_str(),
                    ),
                },
            );
        }
        Ok(Self { providers })
    }

    /// Validate the callback base and non-empty credentials for every declared
    /// provider. Endpoint policy is private runtime state and therefore needs
    /// no operator-config validation branch.
    pub fn validate_at_load(
        &self,
        public_url: &str,
        in_release_build: bool,
    ) -> Result<(), OAuthConfigValidationError> {
        if self.providers.is_empty() {
            return Ok(());
        }
        if parse_public_oauth_base_url(public_url, in_release_build).is_err() {
            let provider = self
                .providers
                .keys()
                .min_by_key(|provider| provider.as_str())
                .map(|provider| provider.as_str().to_owned())
                .unwrap_or_else(|| "<unknown>".to_owned());
            return Err(OAuthConfigValidationError {
                provider,
                reason: "public_url_required",
            });
        }

        let mut providers: Vec<_> = self.providers.iter().collect();
        providers.sort_by_key(|(provider, _)| provider.as_str());
        for (provider, config) in providers {
            config.validate(*provider)?;
        }
        Ok(())
    }
}

impl OAuthProviderConfig {
    fn validate(&self, provider: OAuthProvider) -> Result<(), OAuthConfigValidationError> {
        use secrecy::ExposeSecret;

        if self.client_id.expose_secret().is_empty() {
            return Err(OAuthConfigValidationError {
                provider: provider.as_str().to_owned(),
                reason: "client_id_required",
            });
        }
        if self.client_secret.expose_secret().is_empty() {
            return Err(OAuthConfigValidationError {
                provider: provider.as_str().to_owned(),
                reason: "client_secret_required",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::assert_not_impl_any!(OAuthProvidersConfig: Clone);
    static_assertions::assert_not_impl_any!(OAuthProviderConfig: Clone);

    fn credentials() -> OAuthProviderConfig {
        OAuthProviderConfig {
            client_id: SecretString::new("client".into()),
            client_secret: SecretString::new("secret".into()),
        }
    }

    fn config_with(provider: OAuthProvider) -> OAuthProvidersConfig {
        OAuthProvidersConfig {
            providers: HashMap::from([(provider, credentials())]),
        }
    }

    #[test]
    fn microsoft_declaration_has_secret_free_precedence() {
        const CANARY: &str = "MICROSOFT_CONFIG_CANARY_DO_NOT_ECHO";
        let mut env = nebula_env::testing::EnvGuard::acquire();
        env.set("API_AUTH_OAUTH_GOOGLE_JWKS_URL", "");
        env.set("API_AUTH_OAUTH_MICROSOFT_CLIENT_SECRET", CANARY);

        let error = OAuthProvidersConfig::from_env().expect_err("Microsoft remains parked");
        assert!(matches!(
            error,
            ApiConfigError::OAuthProviderParked {
                provider: "microsoft"
            }
        ));
        assert!(!format!("{error:?} {error}").contains(CANARY));
    }

    #[test]
    fn jwks_declaration_is_rejected_even_when_empty() {
        let mut env = nebula_env::testing::EnvGuard::acquire();
        env.set("API_AUTH_OAUTH_ACME_JWKS_URL", "");

        assert!(matches!(
            OAuthProvidersConfig::from_env(),
            Err(ApiConfigError::OAuthConfigUnsupported {
                reason: "jwks_url_unsupported"
            })
        ));
    }

    #[test]
    fn unknown_provider_is_rejected_without_echoing_value() {
        const CANARY: &str = "UNKNOWN_PROVIDER_CONFIG_CANARY_DO_NOT_ECHO";
        let mut env = nebula_env::testing::EnvGuard::acquire();
        env.set("API_AUTH_OAUTH_GOOOGLE_CLIENT_SECRET", CANARY);

        let error = OAuthProvidersConfig::from_env().expect_err("provider typo must fail");
        assert!(matches!(
            error,
            ApiConfigError::OAuthConfigUnsupported {
                reason: "provider_unknown"
            }
        ));
        assert!(!format!("{error:?} {error}").contains(CANARY));
    }

    #[test]
    fn known_provider_profile_keys_and_global_bypass_are_rejected_even_when_empty() {
        for key in [
            "API_AUTH_OAUTH_GOOGLE_DISCOVERY_URL",
            "API_AUTH_OAUTH_GITHUB_AUTHORIZE_URL",
            "API_AUTH_OAUTH_GITHUB_TOKEN_URL",
            "API_AUTH_OAUTH_GITHUB_TOKEN_ENDPOINT_AUTH_METHOD",
            "API_AUTH_OAUTH_GITHUB_USERINFO_URL",
            "API_AUTH_OAUTH_GITHUB_VERIFIED_EMAILS_URL",
            "API_AUTH_OAUTH_GITHUB_SCOPES",
            "API_AUTH_OAUTH_GOOGLE_FUTURE_PROFILE_FIELD",
            "API_AUTH_OAUTH_ALLOW_INSECURE_LOCALHOST",
        ] {
            let mut env = nebula_env::testing::EnvGuard::acquire();
            env.set(key, "");
            assert!(
                matches!(
                    OAuthProvidersConfig::from_env(),
                    Err(ApiConfigError::OAuthConfigUnsupported {
                        reason: "provider_profile_override_unsupported"
                    })
                ),
                "key={key}"
            );
        }
    }

    #[test]
    fn credentials_only_environment_loads_without_formatting_secrets() {
        use secrecy::ExposeSecret;

        let mut env = nebula_env::testing::EnvGuard::acquire();
        env.set("API_AUTH_OAUTH_GITHUB_CLIENT_ID", " client-id ");
        env.set("API_AUTH_OAUTH_GITHUB_CLIENT_SECRET", " secret-value ");

        let config = OAuthProvidersConfig::from_env().expect("credentials are accepted");
        let github = config
            .providers
            .get(&OAuthProvider::GitHub)
            .expect("GitHub is declared");
        assert_eq!(github.client_id.expose_secret(), " client-id ");
        assert_eq!(github.client_secret.expose_secret(), " secret-value ");
    }

    #[test]
    fn half_declaration_is_retained_and_fails_validation() {
        let mut env = nebula_env::testing::EnvGuard::acquire();
        env.set("API_AUTH_OAUTH_GOOGLE_CLIENT_SECRET", "secret");

        let config = OAuthProvidersConfig::from_env().expect("loader retains half config");
        let error = config
            .validate_at_load("https://nebula.example", true)
            .expect_err("missing client id must fail boot validation");
        assert_eq!(error.provider, "google");
        assert_eq!(error.reason, "client_id_required");
    }

    #[test]
    fn serde_cannot_represent_endpoint_or_profile_overrides() {
        let credential_input = serde_json::json!({
            "client_id": "must-not-be-silently-ignored",
            "client_secret": "must-not-be-silently-ignored"
        });
        assert!(
            serde_json::from_value::<OAuthProviderConfig>(credential_input).is_err(),
            "structured credentials must fail instead of becoming empty placeholders"
        );

        let provider_override = serde_json::json!({
            "client_id": "ignored",
            "client_secret": "ignored",
            "endpoints": {"kind": "oidc", "discovery_url": "https://example.test"}
        });
        assert!(serde_json::from_value::<OAuthProviderConfig>(provider_override).is_err());

        let global_override = serde_json::json!({
            "providers": {},
            "oauth_allow_insecure_localhost": true
        });
        assert!(serde_json::from_value::<OAuthProvidersConfig>(global_override).is_err());
    }

    #[test]
    fn credential_debug_is_redacted() {
        let config = OAuthProvidersConfig {
            providers: HashMap::from([(
                OAuthProvider::GitHub,
                OAuthProviderConfig {
                    client_id: SecretString::new("CLIENT_CANARY".into()),
                    client_secret: SecretString::new("SECRET_CANARY".into()),
                },
            )]),
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("CLIENT_CANARY"));
        assert!(!debug.contains("SECRET_CANARY"));
    }

    #[test]
    fn empty_provider_map_is_a_noop() {
        OAuthProvidersConfig::default()
            .validate_at_load("", false)
            .expect("OAuth remains opt-in");
    }

    #[test]
    fn public_url_failure_uses_stable_provider_label() {
        let mut config = config_with(OAuthProvider::Google);
        config
            .providers
            .insert(OAuthProvider::GitHub, credentials());

        let error = config
            .validate_at_load("", false)
            .expect_err("callback URL is required");
        assert_eq!(error.provider, "github");
        assert_eq!(error.reason, "public_url_required");
    }

    #[test]
    fn public_url_policy_accepts_mount_prefix_and_debug_localhost_only() {
        let config = config_with(OAuthProvider::Google);
        for invalid in [
            "https://user:secret@nebula.example/",
            "https://nebula.example/?tenant=canary",
            "https://nebula.example/#fragment-canary",
            "https://nebula.example/base//path",
            "https://nebula.example/base/../admin",
            "https://nebula.example/base/%2Fescape",
            "https://nebula.example/base\\escape",
            "http://nebula.example/",
            " https://nebula.example",
            "https://nebula.example\n",
            "https://",
            "javascript:alert(1)",
        ] {
            let error = config
                .validate_at_load(invalid, false)
                .expect_err("ambiguous callback base must fail");
            assert_eq!(error.reason, "public_url_required", "raw={invalid}");
        }

        assert!(
            config
                .validate_at_load("http://localhost:8080/", false)
                .is_ok()
        );
        assert!(
            config
                .validate_at_load("http://localhost:8080/", true)
                .is_err()
        );
        for valid in [
            "https://nebula.example/",
            "https://nebula.example/nebula",
            "https://nebula.example/nebula/",
        ] {
            assert!(config.validate_at_load(valid, false).is_ok(), "raw={valid}");
        }
    }
}
