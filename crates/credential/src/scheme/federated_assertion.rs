//! Federated identity assertion (SAML, JWT, Kerberos ticket).

use chrono::{DateTime, Utc};
use nebula_core::{AuthPattern, AuthScheme};
use serde::{Deserialize, Serialize};

use crate::SecretString;

/// A third-party identity assertion such as a SAML assertion, JWT, or
/// Kerberos ticket.
///
/// Unlike `OAuth2Token`, this type carries the raw assertion blob directly
/// rather than a parsed token set. It is consumed by services that accept
/// federated identity proofs as authentication material.
///
/// # Examples
///
/// ```
/// use nebula_credential::{SecretString, scheme::FederatedAssertion};
///
/// let assertion = FederatedAssertion::new(
///     SecretString::new("eyJhbGciOiJSUzI1NiJ9..."),
///     "https://idp.example.com",
/// );
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct FederatedAssertion {
    #[serde(with = "crate::serde_secret")]
    assertion: SecretString,
    issuer: String,
    expires_at: Option<DateTime<Utc>>,
}

impl FederatedAssertion {
    /// Creates a new federated assertion from the given blob and issuer.
    #[must_use]
    pub fn new(assertion: SecretString, issuer: impl Into<String>) -> Self {
        Self {
            assertion,
            issuer: issuer.into(),
            expires_at: None,
        }
    }

    /// Sets the expiry time of the assertion.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_expires_at(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Returns the raw assertion blob.
    pub fn assertion(&self) -> &SecretString {
        &self.assertion
    }

    /// Returns the issuer identifier (URL or name).
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Returns when the assertion expires, if known.
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

impl AuthScheme for FederatedAssertion {
    fn pattern() -> AuthPattern {
        AuthPattern::FederatedIdentity
    }

    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}

impl std::fmt::Debug for FederatedAssertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FederatedAssertion")
            .field("assertion", &"[REDACTED]")
            .field("issuer", &self.issuer)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_federated_identity() {
        assert_eq!(
            FederatedAssertion::pattern(),
            AuthPattern::FederatedIdentity
        );
    }

    #[test]
    fn debug_redacts_assertion() {
        let a = FederatedAssertion::new(
            SecretString::new("eyJhbGciOiJSUzI1NiJ9.secret"),
            "https://idp.example.com",
        );
        let debug = format!("{a:?}");
        assert!(debug.contains("https://idp.example.com"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("eyJhbGciOiJSUzI1NiJ9.secret"));
    }

    #[test]
    fn expires_at_propagates_to_auth_scheme() {
        let expiry = chrono::Utc::now() + chrono::Duration::hours(1);
        let a = FederatedAssertion::new(SecretString::new("tok"), "issuer").with_expires_at(expiry);
        assert_eq!(AuthScheme::expires_at(&a), Some(expiry));
    }
}
