//! Challenge-response protocol credentials (Digest, NTLM, SCRAM).

use nebula_core::{AuthPattern, AuthScheme, SecretString};
use serde::{Deserialize, Serialize};

/// Credentials for challenge-response authentication protocols.
///
/// Covers HTTP Digest Auth, NTLM, SCRAM, and other protocols where the
/// server issues a challenge and the client computes a response from an
/// identity, secret, and the challenge.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::ChallengeSecret;
/// use nebula_core::SecretString;
///
/// let cred = ChallengeSecret::new("alice", SecretString::new("password"), "SCRAM-SHA-256");
/// ```
#[derive(Clone, Serialize, Deserialize)]
pub struct ChallengeSecret {
    identity: String,
    #[serde(with = "nebula_core::serde_secret")]
    secret: SecretString,
    protocol: String,
}

impl ChallengeSecret {
    /// Creates new challenge-response credentials.
    #[must_use]
    pub fn new(
        identity: impl Into<String>,
        secret: SecretString,
        protocol: impl Into<String>,
    ) -> Self {
        Self {
            identity: identity.into(),
            secret,
            protocol: protocol.into(),
        }
    }

    /// Returns the identity used in the challenge-response exchange.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the shared secret used to compute responses.
    pub fn secret(&self) -> &SecretString {
        &self.secret
    }

    /// Returns the protocol name (e.g., `"SCRAM-SHA-256"`, `"NTLM"`, `"Digest"`).
    pub fn protocol(&self) -> &str {
        &self.protocol
    }
}

impl AuthScheme for ChallengeSecret {
    fn pattern() -> AuthPattern {
        AuthPattern::ChallengeResponse
    }
}

impl std::fmt::Debug for ChallengeSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChallengeSecret")
            .field("identity", &self.identity)
            .field("secret", &"[REDACTED]")
            .field("protocol", &self.protocol)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_challenge_response() {
        assert_eq!(ChallengeSecret::pattern(), AuthPattern::ChallengeResponse);
    }

    #[test]
    fn debug_redacts_secret() {
        let cred = ChallengeSecret::new("alice", SecretString::new("pa$$w0rd"), "SCRAM-SHA-256");
        let debug = format!("{cred:?}");
        assert!(debug.contains("alice"));
        assert!(debug.contains("SCRAM-SHA-256"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("pa$$w0rd"));
    }
}
