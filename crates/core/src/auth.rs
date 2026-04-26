//! Authentication scheme contract types and pattern classification.
//!
//! `AuthScheme` is the bridge between the credential system and the
//! resource system. Resources declare what auth material they need
//! (`type Auth: AuthScheme`), and credentials produce it via `project()`.
//!
//! `AuthPattern` groups auth schemes into universal categories for UI,
//! logging, and tooling.
//!
//! # Sensitivity dichotomy (§15.5)
//!
//! `AuthScheme` is the base trait — it carries no security guarantees by
//! itself. Implementing types declare sensitivity by also implementing
//! one of:
//!
//! - `SensitiveScheme: AuthScheme + ZeroizeOnDrop` — schemes that hold secret material (tokens,
//!   passwords, keys, certificate private keys).
//! - `PublicScheme: AuthScheme` — schemes that hold no secret material (provider/role/region
//!   identifiers, public capability descriptors).
//!
//! A scheme MUST implement exactly one of these. The `#[derive(AuthScheme)]`
//! macro accepts `#[auth_scheme(sensitive)]` or `#[auth_scheme(public)]`
//! to declare the sensitivity and audit fields at expansion time.

use serde::{Deserialize, Serialize};
use zeroize::ZeroizeOnDrop;

// ── AuthPattern ─────────────────────────────────────────────────────────────

/// Classification of authentication patterns.
///
/// 10 built-in patterns cover common integration auth mechanisms.
/// `Custom` handles everything else.
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

/// Base trait for runtime scheme output.
///
/// Implementations are concrete structs holding scheme material. Sensitivity
/// is declared by the implementing crate via the `SensitiveScheme` or
/// `PublicScheme` sub-trait — these are mutually exclusive and non-optional
/// for any scheme that ships in production code.
///
/// `Clone` is NOT a supertrait — per Tech Spec §15.2, schemes opt in to
/// `Clone` only when copying plaintext is acceptable for the type. Pattern:
/// long-lived consumers receive `SchemeGuard` (per §15.7), not raw clones.
///
/// `Serialize` / `DeserializeOwned` are NOT supertraits — schemes that need
/// to round-trip through storage opt in via concrete `serde` derives. The
/// reduction here closes security-lead N2 by removing the implicit "every
/// scheme can be serialized into telemetry" assumption.
pub trait AuthScheme: Send + Sync + 'static {
    /// Classification for UI, logging, and tooling.
    fn pattern() -> AuthPattern;
}

/// Schemes that hold secret material.
///
/// Mandates [`ZeroizeOnDrop`] so plaintext drops from the heap deterministically.
/// Derived via `#[auth_scheme(sensitive)]`; the macro audits fields at
/// expansion to forbid plain `String` / `Vec<u8>` for token-named slots.
///
/// Examples: `BearerScheme`, `BasicScheme`, `OAuth2Token`, `KeyPair`,
/// `Certificate`, `SigningKey`, `ConnectionUri`, `SharedKey`.
pub trait SensitiveScheme: AuthScheme + ZeroizeOnDrop {}

/// Schemes that hold no secret material.
///
/// Provider / role / region identifiers, public capability descriptors —
/// anything safe to serialize, log, or display in a UI without redaction.
/// Mutually exclusive with [`SensitiveScheme`] — the derive macro forbids
/// declaring both.
///
/// Examples: `InstanceBinding` (provider + role + region; cloud IMDS lookup
/// happens at runtime, no stored secret).
pub trait PublicScheme: AuthScheme {}

/// No authentication required.
impl AuthScheme for () {
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

/// `()` carries no secret material — it is `PublicScheme` by definition.
impl PublicScheme for () {}

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

    /// `TestToken` exercises the manual `AuthScheme` + `SensitiveScheme`
    /// path — it derives `Zeroize`+`ZeroizeOnDrop` to satisfy
    /// `SensitiveScheme: AuthScheme + ZeroizeOnDrop`.
    #[derive(zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
    struct TestToken {
        value: String,
    }

    impl AuthScheme for TestToken {
        fn pattern() -> AuthPattern {
            AuthPattern::SecretToken
        }
    }

    impl SensitiveScheme for TestToken {}

    #[test]
    fn custom_scheme_reports_correct_pattern() {
        let _t = TestToken { value: "x".into() };
        assert_eq!(TestToken::pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn unit_scheme_pattern_is_no_auth() {
        assert_eq!(<() as AuthScheme>::pattern(), AuthPattern::NoAuth);
    }
}
