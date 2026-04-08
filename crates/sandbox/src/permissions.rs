//! Permission model for sandboxed plugins.
//!
//! Each WASM plugin gets a [`PluginPermissions`] that controls what it can access.
//! Permissions are configured per-plugin and enforced by the sandbox at runtime.
//!
//! By default, a plugin has **no permissions** — it can only do pure computation.

use serde::{Deserialize, Serialize};

/// Permissions granted to a sandboxed plugin.
///
/// Default = deny all. Every capability must be explicitly granted.
///
/// # Examples
///
/// ```
/// use nebula_sandbox::permissions::PluginPermissions;
///
/// // Minimal: pure computation, no I/O
/// let perms = PluginPermissions::none();
///
/// // HTTP to specific domains only
/// let perms = PluginPermissions::none()
///     .with_network(NetworkPermission::allow_domains(["slack.com", "api.slack.com"]));
///
/// // Full access (for official/trusted plugins)
/// let perms = PluginPermissions::all();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPermissions {
    /// Network access control.
    pub network: NetworkPermission,
    /// Filesystem access control.
    pub filesystem: FilesystemPermission,
    /// Environment variable access.
    pub env: EnvPermission,
    /// Which credential keys this plugin can access.
    pub credentials: CredentialPermission,
}

impl PluginPermissions {
    /// No permissions — pure computation only.
    #[must_use]
    pub fn none() -> Self {
        Self {
            network: NetworkPermission::Deny,
            filesystem: FilesystemPermission::Deny,
            env: EnvPermission::Deny,
            credentials: CredentialPermission::Deny,
        }
    }

    /// All permissions — for trusted/official plugins.
    #[must_use]
    pub fn all() -> Self {
        Self {
            network: NetworkPermission::AllowAll,
            filesystem: FilesystemPermission::AllowAll,
            env: EnvPermission::AllowAll,
            credentials: CredentialPermission::AllowAll,
        }
    }

    /// Set network permissions.
    #[must_use]
    pub fn with_network(mut self, network: NetworkPermission) -> Self {
        self.network = network;
        self
    }

    /// Set filesystem permissions.
    #[must_use]
    pub fn with_filesystem(mut self, filesystem: FilesystemPermission) -> Self {
        self.filesystem = filesystem;
        self
    }

    /// Set environment variable permissions.
    #[must_use]
    pub fn with_env(mut self, env: EnvPermission) -> Self {
        self.env = env;
        self
    }

    /// Set credential permissions.
    #[must_use]
    pub fn with_credentials(mut self, credentials: CredentialPermission) -> Self {
        self.credentials = credentials;
        self
    }
}

impl Default for PluginPermissions {
    fn default() -> Self {
        Self::none()
    }
}

// ── Network ──────────────────────────────────────────────────────────────

/// Controls what network requests a plugin can make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NetworkPermission {
    /// No network access.
    Deny,
    /// Unrestricted network access.
    AllowAll,
    /// Only allow requests to specific domains.
    AllowDomains {
        /// Allowed domain patterns (e.g., "slack.com", "*.googleapis.com").
        domains: Vec<String>,
    },
    /// Only allow requests matching specific URL prefixes.
    AllowUrls {
        /// Allowed URL prefixes (e.g., "https://api.slack.com/").
        prefixes: Vec<String>,
    },
}

impl NetworkPermission {
    /// Allow access to specific domains.
    pub fn allow_domains<I, S>(domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::AllowDomains {
            domains: domains.into_iter().map(Into::into).collect(),
        }
    }

    /// Allow access to specific URL prefixes.
    pub fn allow_urls<I, S>(prefixes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::AllowUrls {
            prefixes: prefixes.into_iter().map(Into::into).collect(),
        }
    }

    /// Check if a URL is allowed by this permission.
    pub fn check_url(&self, url: &str) -> bool {
        match self {
            Self::Deny => false,
            Self::AllowAll => true,
            Self::AllowDomains { domains } => {
                let host = extract_host(url);
                domains.iter().any(|d| match_domain(&host, d))
            }
            Self::AllowUrls { prefixes } => prefixes.iter().any(|p| url.starts_with(p)),
        }
    }
}

// ── Filesystem ───────────────────────────────────────────────────────────

/// Controls filesystem access for a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FilesystemPermission {
    /// No filesystem access.
    Deny,
    /// Full filesystem access (dangerous).
    AllowAll,
    /// Read-only access to specific paths.
    ReadOnly {
        /// Allowed paths.
        paths: Vec<String>,
    },
    /// Read-write access to specific paths.
    ReadWrite {
        /// Allowed paths.
        paths: Vec<String>,
    },
}

// ── Environment ──────────────────────────────────────────────────────────

/// Controls environment variable access for a plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EnvPermission {
    /// No env var access.
    Deny,
    /// Full env var access.
    AllowAll,
    /// Only allow specific env var names.
    AllowKeys {
        /// Allowed variable names.
        keys: Vec<String>,
    },
}

// ── Credentials ──────────────────────────────────────────────────────────

/// Controls which credentials a plugin can access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialPermission {
    /// No credential access.
    Deny,
    /// Can access any credential.
    AllowAll,
    /// Can only access credentials with these keys.
    AllowKeys {
        /// Allowed credential keys (e.g., "slack_oauth2").
        keys: Vec<String>,
    },
}

impl CredentialPermission {
    /// Check if access to a credential key is allowed.
    pub fn check_key(&self, key: &str) -> bool {
        match self {
            Self::Deny => false,
            Self::AllowAll => true,
            Self::AllowKeys { keys } => keys.iter().any(|k| k == key),
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Extract host from a URL string.
fn extract_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Match a host against a domain pattern.
/// Supports wildcard prefix: "*.example.com" matches "api.example.com".
fn match_domain(host: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_deny_all() {
        let perms = PluginPermissions::default();
        assert!(!perms.network.check_url("https://example.com"));
        assert!(!perms.credentials.check_key("any_key"));
    }

    #[test]
    fn all_allows_everything() {
        let perms = PluginPermissions::all();
        assert!(perms.network.check_url("https://anything.com"));
        assert!(perms.credentials.check_key("any_key"));
    }

    #[test]
    fn network_domain_allowlist() {
        let net = NetworkPermission::allow_domains(["slack.com", "*.googleapis.com"]);
        assert!(net.check_url("https://slack.com/api/chat"));
        assert!(net.check_url("https://storage.googleapis.com/bucket"));
        assert!(!net.check_url("https://evil.com/steal"));
    }

    #[test]
    fn network_url_prefix_allowlist() {
        let net = NetworkPermission::allow_urls(["https://api.slack.com/"]);
        assert!(net.check_url("https://api.slack.com/chat.postMessage"));
        assert!(!net.check_url("https://api.slack.com"));
        assert!(!net.check_url("https://evil.com"));
    }

    #[test]
    fn credential_allowlist() {
        let cred = CredentialPermission::AllowKeys {
            keys: vec!["slack_oauth2".into()],
        };
        assert!(cred.check_key("slack_oauth2"));
        assert!(!cred.check_key("aws_secret"));
    }

    #[test]
    fn extract_host_works() {
        assert_eq!(extract_host("https://api.slack.com/v1"), "api.slack.com");
        assert_eq!(extract_host("http://localhost:8080/path"), "localhost");
        assert_eq!(extract_host("https://example.com"), "example.com");
    }

    #[test]
    fn wildcard_domain_matching() {
        assert!(match_domain("api.slack.com", "*.slack.com"));
        assert!(match_domain("slack.com", "*.slack.com"));
        assert!(!match_domain("evil-slack.com", "*.slack.com"));
        assert!(match_domain("slack.com", "slack.com"));
        assert!(!match_domain("api.slack.com", "slack.com"));
    }
}
