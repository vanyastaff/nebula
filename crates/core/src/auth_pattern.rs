//! Classification of authentication patterns.
//!
//! [`AuthPattern`] groups auth schemes into universal categories for UI,
//! logging, and tooling. Each [`AuthScheme`](super::AuthScheme) implementation
//! declares its pattern via [`AuthScheme::pattern()`](super::AuthScheme::pattern).

use serde::{Deserialize, Serialize};

/// Classification of authentication patterns.
///
/// 13 built-in patterns cover the vast majority of auth mechanisms.
/// [`Custom`](AuthPattern::Custom) handles everything else.
///
/// # Examples
///
/// ```
/// use nebula_core::AuthPattern;
///
/// let pattern = AuthPattern::OAuth2;
/// assert_eq!(format!("{pattern:?}"), "OAuth2");
/// ```
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
    /// Third-party identity assertion (SAML, JWT, Kerberos ticket).
    FederatedIdentity,
    /// Challenge-response protocol credentials (Digest, NTLM, SCRAM).
    ChallengeResponse,
    /// TOTP/HOTP seed or OTP delivery config.
    OneTimePasscode,
    /// Compound connection URI (postgres://..., redis://...).
    ConnectionUri,
    /// Cloud/infrastructure instance identity (IMDS, managed identity).
    InstanceIdentity,
    /// Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).
    SharedSecret,
    /// Plugin-defined pattern not covered by built-in categories.
    Custom,
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
            AuthPattern::FederatedIdentity,
            AuthPattern::ChallengeResponse,
            AuthPattern::OneTimePasscode,
            AuthPattern::ConnectionUri,
            AuthPattern::InstanceIdentity,
            AuthPattern::SharedSecret,
            AuthPattern::Custom,
        ];
        let set: std::collections::HashSet<_> = variants.iter().collect();
        assert_eq!(set.len(), 14);
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
}
