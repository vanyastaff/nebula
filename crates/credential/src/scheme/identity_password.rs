//! Identity + password authentication (username/email + password).

use nebula_core::SecretString;
use serde::{Deserialize, Serialize};

use crate::{AuthScheme, identity_state};

/// Identity (username, email, or account) paired with a password.
///
/// Covers HTTP Basic Auth, database logins, SSH password auth, and any
/// other scheme that combines an identity string with a secret password.
///
/// # Examples
///
/// ```
/// use nebula_core::SecretString;
/// use nebula_credential::scheme::IdentityPassword;
///
/// let cred = IdentityPassword::new("alice@example.com", SecretString::new("hunter2"));
/// ```
#[derive(Clone, Serialize, Deserialize, AuthScheme)]
#[auth_scheme(pattern = IdentityPassword)]
pub struct IdentityPassword {
    identity: String,
    #[serde(with = "nebula_core::serde_secret")]
    password: SecretString,
}

impl IdentityPassword {
    /// Creates a new identity/password credential.
    #[must_use]
    pub fn new(identity: impl Into<String>, password: SecretString) -> Self {
        Self {
            identity: identity.into(),
            password,
        }
    }

    /// Returns the identity (username, email, or account name).
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the password secret.
    pub fn password(&self) -> &SecretString {
        &self.password
    }
}

// Static credentials use State = Scheme (identity projection).
identity_state!(IdentityPassword, "identity_password", 1);

impl std::fmt::Debug for IdentityPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityPassword")
            .field("identity", &self.identity)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{AuthPattern, AuthScheme as _};

    use super::*;

    #[test]
    fn pattern_is_identity_password() {
        assert_eq!(IdentityPassword::pattern(), AuthPattern::IdentityPassword);
    }

    #[test]
    fn debug_redacts_password() {
        let cred = IdentityPassword::new("alice", SecretString::new("s3cr3t"));
        let debug = format!("{cred:?}");
        assert!(debug.contains("alice"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("s3cr3t"));
    }
}
