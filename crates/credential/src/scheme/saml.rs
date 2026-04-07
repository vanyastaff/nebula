//! SAML assertion authentication.

use std::collections::HashMap;

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use nebula_core::SecretString;

/// SAML assertion authentication material.
///
/// Produced by: SAML IdP authentication flows.
/// Consumed by: Service providers requiring SAML assertions.
#[derive(Clone, Serialize, Deserialize)]
pub struct SamlAuth {
    /// SAML NameID (subject identifier).
    pub name_id: String,
    /// SAML attributes (key -> multi-valued).
    pub attributes: HashMap<String, Vec<String>>,
    /// Session index for single logout.
    pub session_index: Option<String>,
    /// Assertion expiration time.
    pub not_on_or_after: Option<chrono::DateTime<chrono::Utc>>,
    /// Base64-encoded SAML assertion (secret -- may contain sensitive claims).
    #[serde(with = "nebula_core::option_serde_secret")]
    assertion_b64: Option<SecretString>,
}

impl SamlAuth {
    /// Creates a new SAML auth with the given name ID.
    pub fn new(name_id: impl Into<String>) -> Self {
        Self {
            name_id: name_id.into(),
            attributes: HashMap::new(),
            session_index: None,
            not_on_or_after: None,
            assertion_b64: None,
        }
    }

    /// Sets the SAML attributes.
    pub fn with_attributes(mut self, attributes: HashMap<String, Vec<String>>) -> Self {
        self.attributes = attributes;
        self
    }

    /// Sets the session index.
    pub fn with_session_index(mut self, index: impl Into<String>) -> Self {
        self.session_index = Some(index.into());
        self
    }

    /// Sets the assertion expiration time.
    pub fn with_not_on_or_after(mut self, at: chrono::DateTime<chrono::Utc>) -> Self {
        self.not_on_or_after = Some(at);
        self
    }

    /// Sets the base64-encoded SAML assertion.
    pub fn with_assertion(mut self, assertion: SecretString) -> Self {
        self.assertion_b64 = Some(assertion);
        self
    }

    /// Returns the base64-encoded SAML assertion, if present.
    pub fn assertion_b64(&self) -> Option<&SecretString> {
        self.assertion_b64.as_ref()
    }
}

impl AuthScheme for SamlAuth {
    const KIND: &'static str = "saml";

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.not_on_or_after
    }
}

impl std::fmt::Debug for SamlAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamlAuth")
            .field("name_id", &self.name_id)
            .field("attributes", &self.attributes)
            .field("session_index", &self.session_index)
            .field("not_on_or_after", &self.not_on_or_after)
            .field(
                "assertion_b64",
                if self.assertion_b64.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(SamlAuth::KIND, "saml");
    }

    #[test]
    fn debug_redacts_assertion() {
        let auth =
            SamlAuth::new("user@example.com").with_assertion(SecretString::new("PHNhbWw+..."));
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("PHNhbWw+"));
    }

    #[test]
    fn expires_at_returns_not_on_or_after() {
        let expiry = chrono::Utc::now() + chrono::Duration::hours(1);
        let auth = SamlAuth::new("user@example.com").with_not_on_or_after(expiry);
        assert_eq!(auth.expires_at(), Some(expiry));
    }

    #[test]
    fn expires_at_returns_none_when_unset() {
        let auth = SamlAuth::new("user@example.com");
        assert_eq!(auth.expires_at(), None);
    }
}
