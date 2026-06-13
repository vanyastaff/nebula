//! Mechanics families for the built-in auth schemes (F3).
//!
//! Each built-in scheme declares one of these as its
//! [`AuthScheme::Family`](nebula_core::auth::AuthScheme::Family). A family is a
//! zero-sized marker carrying the *mechanics* of the scheme — the wire-egress
//! shape(s) it presents and the renewal strategies it may legitimately use —
//! on the open [`SchemeFamily`] axis, decoupled from the cosmetic
//! [`AuthPattern`]. Plugins define their own family types the same way, with
//! zero framework edits.
//!
//! The `EGRESS` slice and `refresh_classes` set on each family are the
//! canonical, contract-frozen mechanics of the corresponding built-in.

use nebula_core::auth::{AuthPattern, EgressShape, RefreshStrategy, SchemeFamily};

/// Macro: define a zero-sized built-in family marker + its `SchemeFamily` impl.
macro_rules! builtin_family {
    (
        $(#[$meta:meta])*
        $name:ident => egress: [$($egress:ident),+ $(,)?],
                       refresh: [$($refresh:ident),+ $(,)?],
                       pattern: $pattern:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct $name;

        impl SchemeFamily for $name {
            const EGRESS: &'static [EgressShape] = &[$(EgressShape::$egress),+];
            fn refresh_classes() -> &'static [RefreshStrategy] {
                &[$(RefreshStrategy::$refresh),+]
            }
            fn pattern() -> AuthPattern {
                AuthPattern::$pattern
            }
        }
    };
}

builtin_family! {
    /// Opaque secret token sent inline (API key, PAT, bearer) — never renewed.
    SecretTokenFamily => egress: [InlineSecret], refresh: [Static], pattern: SecretToken
}

builtin_family! {
    /// Identity + password sent inline (HTTP Basic, account credentials).
    IdentityPasswordFamily => egress: [InlineSecret], refresh: [Static], pattern: IdentityPassword
}

builtin_family! {
    /// OAuth2 access token sent inline; renews via refresh-token grant, or
    /// re-acquires when no refresh token was issued.
    OAuth2Family => egress: [InlineSecret], refresh: [RefreshToken, ReAcquire], pattern: OAuth2
}

builtin_family! {
    /// X.509 / mTLS client certificate presented in the handshake; re-acquired
    /// (re-issued) rather than incrementally refreshed.
    CertificateFamily => egress: [CertPresentation], refresh: [ReAcquire], pattern: Certificate
}

builtin_family! {
    /// Compound connection URI whose secret is presented at connection setup
    /// (database / message-queue DSN).
    ConnectionUriFamily => egress: [ConnectionHandshakeSecret], refresh: [Static], pattern: ConnectionUri
}

builtin_family! {
    /// Request-signing key (HMAC, AWS SigV4) — the secret signs the outbound
    /// request and never leaves the signer.
    SigningKeyFamily => egress: [SignedRequest], refresh: [Static], pattern: RequestSigning
}

builtin_family! {
    /// Asymmetric key pair presented as a certificate/public key (SSH, PGP).
    KeyPairFamily => egress: [CertPresentation], refresh: [Static], pattern: KeyPair
}

builtin_family! {
    /// Pre-shared symmetric key that keys a transport session (TLS-PSK, WireGuard).
    SharedKeyFamily => egress: [KeyAgreement], refresh: [Static], pattern: SharedSecret
}

builtin_family! {
    /// Cloud instance identity resolved from a metadata service; ambient
    /// material is re-acquired, and the terminal egress is a signed request.
    InstanceBindingFamily => egress: [SignedRequest], refresh: [ReAcquire], pattern: InstanceIdentity
}
