//! Authentication scheme contract types and pattern classification.
//!
//! [`AuthScheme`] is the bridge between the credential system and the
//! resource system. Resources declare what auth material they need
//! (`type Auth: AuthScheme`), and credentials produce it via `project()`.
//!
//! [`AuthPattern`] groups auth schemes into universal categories for UI,
//! logging, and tooling.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

// ── AuthPattern ─────────────────────────────────────────────────────────────

/// Classification of authentication patterns.
///
/// 10 built-in patterns cover common integration auth mechanisms.
/// [`Custom`](AuthPattern::Custom) handles everything else.
///
/// **Pruned 2026-04-24** (zero consumers, Plane-A territory):
/// `FederatedIdentity` (SAML/JWT → `nebula-auth`, not integration credentials),
/// `ChallengeResponse` (Digest/NTLM/SCRAM — HTTP client negotiation),
/// `OneTimePasscode` (TOTP/HOTP — integration-internal, not projected auth).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AuthPattern {
    /// No authentication required.
    NoAuth,
    /// Opaque secret string (API key, bearer token, session token).
    SecretToken,
    /// Identity + password pair (user/email/account + password).
    IdentityPassword,
    /// OAuth2/OIDC token set.
    OAuth2,
    /// Asymmetric key pair (SSH, PGP, crypto wallets).
    KeyPair,
    /// X.509 certificate + private key (mTLS, TLS client auth).
    Certificate,
    /// Request signing credentials (HMAC, SigV4, webhook signatures).
    RequestSigning,
    /// Compound connection URI (postgres://..., redis://...).
    ConnectionUri,
    /// Cloud/infrastructure instance identity (IMDS, managed identity).
    InstanceIdentity,
    /// Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).
    SharedSecret,
    /// Plugin-defined pattern not covered by built-in categories.
    Custom,
}

// ── AuthScheme ──────────────────────────────────────────────────────────────

/// Consumer-facing authentication material.
///
/// Resources declare `type Auth: AuthScheme` to specify what auth
/// material they need. Credentials produce it via `Credential::project()`.
///
/// # Security contract
///
/// `Serialize + DeserializeOwned` bounds exist for the State = Scheme
/// identity path (static credentials stored directly). Serialization
/// to plaintext JSON happens **exclusively** inside `EncryptionLayer`.
/// Never serialize `AuthScheme` types in logging, debugging, or telemetry.
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Classification for UI, logging, and tooling.
    fn pattern() -> AuthPattern;

    /// When this auth material expires, if applicable.
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        None
    }
}

/// No authentication required.
impl AuthScheme for () {
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_are_distinct() {
        let variants = [
            AuthPattern::NoAuth,
            AuthPattern::SecretToken,
            AuthPattern::IdentityPassword,
            AuthPattern::OAuth2,
            AuthPattern::KeyPair,
            AuthPattern::Certificate,
            AuthPattern::RequestSigning,
            AuthPattern::ConnectionUri,
            AuthPattern::InstanceIdentity,
            AuthPattern::SharedSecret,
            AuthPattern::Custom,
        ];
        let set: std::collections::HashSet<_> = variants.iter().collect();
        assert_eq!(set.len(), 11);
    }

    #[test]
    fn serde_round_trips() {
        let pattern = AuthPattern::OAuth2;
        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: AuthPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(pattern, deserialized);
    }

    #[test]
    fn debug_output_is_readable() {
        assert_eq!(format!("{:?}", AuthPattern::SecretToken), "SecretToken");
    }

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    struct TestToken {
        value: String,
    }

    impl AuthScheme for TestToken {
        fn pattern() -> AuthPattern {
            AuthPattern::SecretToken
        }
    }

    #[test]
    fn custom_scheme_reports_correct_pattern() {
        assert_eq!(TestToken::pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn unit_scheme_pattern_is_no_auth() {
        assert_eq!(<() as AuthScheme>::pattern(), AuthPattern::NoAuth);
    }
}
