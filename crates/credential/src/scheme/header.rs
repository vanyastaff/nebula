//! Custom HTTP header authentication.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// Authentication via a custom HTTP header.
///
/// Produced by: Header-based API key credentials.
/// Consumed by: APIs that use non-standard auth headers (e.g., `X-Api-Key`).
#[derive(Clone, Serialize, Deserialize)]
pub struct HeaderAuth {
    /// Header name (e.g., `"X-Api-Key"`).
    pub name: String,
    /// Header value (secret).
    value: SecretString,
}

impl HeaderAuth {
    /// Creates a new header auth with the given name and secret value.
    pub fn new(name: impl Into<String>, value: SecretString) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// Returns the header name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the header value secret.
    pub fn value(&self) -> &SecretString {
        &self.value
    }
}

impl AuthScheme for HeaderAuth {
    const KIND: &'static str = "header";
}

impl std::fmt::Debug for HeaderAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeaderAuth")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(HeaderAuth::KIND, "header");
    }

    #[test]
    fn debug_redacts_secrets() {
        let auth = HeaderAuth::new("X-Api-Key", SecretString::new("super-secret"));
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn accessors_return_expected_values() {
        let auth = HeaderAuth::new("X-Api-Key", SecretString::new("val"));
        assert_eq!(auth.name(), "X-Api-Key");
        auth.value().expose_secret(|v| assert_eq!(v, "val"));
    }
}
