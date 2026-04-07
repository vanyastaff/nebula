//! Opaque secret token authentication (API key, bearer token, session token).

use nebula_core::SecretString;

use crate::AuthScheme; // derive macro
use crate::identity_state;
use serde::{Deserialize, Serialize};

/// An opaque secret string used as an authentication token.
///
/// Covers API keys, pre-issued bearer tokens, session tokens, and any other
/// single-value opaque credential.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::SecretToken;
/// use nebula_core::SecretString;
///
/// let token = SecretToken::new(SecretString::new("sk-abc123"));
/// ```
#[derive(Clone, Serialize, Deserialize, AuthScheme)]
#[auth_scheme(pattern = SecretToken)]
pub struct SecretToken {
    #[serde(with = "nebula_core::serde_secret")]
    token: SecretString,
}

impl SecretToken {
    /// Creates a new secret token.
    #[must_use]
    pub fn new(token: SecretString) -> Self {
        Self { token }
    }

    /// Returns the secret token value.
    pub fn token(&self) -> &SecretString {
        &self.token
    }
}

// Static credentials use State = Scheme (identity projection).
identity_state!(SecretToken, "secret_token", 1);

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretToken")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{AuthPattern, AuthScheme as _};

    use super::*;

    #[test]
    fn pattern_is_secret_token() {
        assert_eq!(SecretToken::pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn debug_redacts_token() {
        let t = SecretToken::new(SecretString::new("sk-super-secret"));
        let debug = format!("{t:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("sk-super-secret"));
    }
}
