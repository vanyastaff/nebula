//! Authentication scheme contract types and pattern classification.
//!
//! `AuthScheme` is the bridge between the credential system and the
//! resource system. A credential declares the scheme it yields via its
//! `Credential::Scheme: AuthScheme` associated type and produces it via
//! `project()`; a resource consumes that scheme as a typed
//! `CredentialGuard<Scheme>` bound into a slot, so a cross-protocol bind is a
//! nominal compile error.
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
    /// The mechanics [`SchemeFamily`] this scheme belongs to — the open axis of
    /// the F3 model. A novel protocol is a new family *type* with zero framework
    /// `match`; the family declares the wire-egress shape(s) and the legitimate
    /// renewal strategies, checked against the credential's policy at registration.
    type Family: SchemeFamily;

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
///
/// ## Exclusivity with `PublicScheme` is macro-enforced, not trait-enforced
///
/// `SensitiveScheme: AuthScheme + ZeroizeOnDrop` and
/// `PublicScheme: AuthScheme` — both are satisfiable simultaneously by
/// the same concrete type. Rust does not currently expose a stable
/// negative-impl mechanism (`impl !PublicScheme for X`), so the
/// dichotomy is enforced **at the `#[derive(AuthScheme)]` macro layer**:
/// `#[auth_scheme(sensitive, public)]` is a compile error, and the
/// macro emits exactly one of `impl SensitiveScheme` or
/// `impl PublicScheme`. Hand-rolled impls bypass that audit — a
/// downstream `impl SensitiveScheme for X { } impl PublicScheme for X { }`
/// pair will type-check.
///
/// Defense-in-depth: the `ZeroizeOnDrop` bound catches a
/// `SensitiveScheme` impl on a struct that doesn't zeroize (the
/// canonical safety invariant), so even with two impls the sensitive
/// bound carries the safety guarantee. See
/// `arch-publicscheme-nested-sensitive-audit` in
/// `docs/tracking/credential-concerns-register.md` for the long-term
/// refinement plan (sealed `Sensitivity` associated tag, or signed
/// manifests that surface dual impls at registry time).
pub trait SensitiveScheme: AuthScheme + ZeroizeOnDrop {}

/// Schemes that hold no secret material.
///
/// Provider / role / region identifiers, public capability descriptors —
/// anything safe to serialize, log, or display in a UI without redaction.
///
/// ## Exclusivity with `SensitiveScheme`
///
/// **Macro-enforced**, not trait-enforced. The derive macro
/// (`#[auth_scheme(public)]` vs `#[auth_scheme(sensitive)]`) forbids
/// declaring both, but a hand-rolled `impl PublicScheme for X` on a
/// type that also `impl SensitiveScheme for X` will compile — there is
/// no stable negative-impl mechanism in Rust today. See
/// [`SensitiveScheme`] doc-comment for the full discussion and pointer
/// to the tracking concerns row.
///
/// Examples: `InstanceBinding` (provider + role + region; cloud IMDS lookup
/// happens at runtime, no stored secret).
pub trait PublicScheme: AuthScheme {}

/// The mechanics family of the no-auth scheme: nothing crosses the wire, nothing
/// is renewed. The `Family` of `()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoAuthFamily;

impl SchemeFamily for NoAuthFamily {
    const EGRESS: &'static [EgressShape] = &[EgressShape::None];
    fn refresh_classes() -> &'static [RefreshStrategy] {
        &[RefreshStrategy::Static]
    }
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

/// No authentication required.
impl AuthScheme for () {
    type Family = NoAuthFamily;
    fn pattern() -> AuthPattern {
        AuthPattern::NoAuth
    }
}

/// `()` carries no secret material — it is `PublicScheme` by definition.
impl PublicScheme for () {}

// ── Mechanics axis (F3) ───────────────────────────────────────────────────────

/// How a credential's secret material relates to the wire — the **one closed
/// set** in the scheme model.
///
/// Validated complete for the 2026 protocol universe across eight transport
/// domains (~150 mechanisms reduced to these primitives): there is no further
/// physical way for a secret to relate to a wire. Deliberately a sealed
/// `#[non_exhaustive]` enum, **not** an open trait: the framework must `match`
/// the egress shape to drive redaction, the SSRF-hardened refresh transport,
/// audit, and observability, so an open egress would let a plugin declare a
/// shape those consumers have never seen — a secret-leak-by-open-world. Adding a
/// variant is therefore a deliberate framework edit shipped *atomically* with
/// its redaction/transport handler. (The open-world axis is [`SchemeFamily`],
/// not this.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EgressShape {
    /// Nothing secret crosses the wire — true mTLS-only-no-secret, object-capability
    /// bearing, or an inbound-verification secret used to *verify* (not send).
    None,
    /// Secret bytes carried inline in a request/auth field — transport-agnostic:
    /// HTTP `Authorization` header, SASL OAUTHBEARER blob, gRPC metadata, SOAP
    /// `<wsse:Security>`, OPC-UA identity token, Cap'n Proto SturdyRef.
    InlineSecret,
    /// Secret presented at connection establishment — a static DB password or an
    /// ephemeral token-as-password (RDS-IAM, Azure-Entra, MongoDB-OIDC, Redis-Entra).
    ConnectionHandshakeSecret,
    /// A signature/MAC over the *outbound request*; the secret never leaves the
    /// signer — symmetric (HMAC, AWS SigV4) or asymmetric (SigV4a,
    /// `private_key_jwt`), over any transport. The carrier (header/query/body) is
    /// a sub-field, not a variant.
    SignedRequest,
    /// RFC 9421 HTTP Message Signatures — specifically. Narrow by design; does not
    /// absorb SASL/GSSAPI handshake proofs (those are [`Self::ChallengeResponse`]).
    NegotiatedSignature,
    /// Present a structured identity certificate in a handshake — X.509/mTLS, an
    /// SSH certificate, a raw public key (RFC 7250), or a SPIFFE SVID.
    CertPresentation,
    /// Sender-constrained / bound token: a bearer token *plus* a binding proof, as
    /// one credential — DPoP (RFC 9449), mTLS-bound tokens (RFC 8705), SAML
    /// Holder-of-Key. Two simultaneous shapes are expressed by a multi-element
    /// [`SchemeFamily::EGRESS`].
    ProofOfPossession,
    /// The secret is a key used to derive a per-exchange proof inside a (possibly
    /// multi-round) connection/SASL handshake; the secret itself is never sent —
    /// SCRAM, CRAM-MD5, MySQL native/caching_sha2, NTLM, Kerberos/GSSAPI/SPNEGO,
    /// SNMPv3-auth, RADIUS, NATS nkey, ALTS.
    ChallengeResponse,
    /// A symmetric pre-shared key or static keypair keys / authenticates /
    /// protects a transport session or message body; never transmitted —
    /// WireGuard, Noise, TLS-PSK / DTLS-PSK, IKEv2-PSK, LoRaWAN, SNMPv3-privacy.
    KeyAgreement,
    /// An external signer produces the wire credential and the framework holds no
    /// secret bytes, only a handle — ssh-agent, TPM, HSM/PKCS#11, FIDO2/WebAuthn,
    /// cloud KMS, Secure Enclave, Ledger/Trezor. Pairs with a `External`-sensitivity
    /// scheme.
    DelegatedSignature,
    /// The credential signs *caller-supplied* bytes the caller then broadcasts;
    /// neither the secret nor the signature crosses a framework-owned wire — raw
    /// ECDSA/ed25519 transaction signing, HD-wallet keys, SAML IdP/SP XML-DSig.
    DetachedSignature,
}

/// The **mechanics** of an auth scheme — the open axis of the F3 model.
///
/// Unlike [`EgressShape`] (a sealed primitive set), `SchemeFamily` is an **open
/// trait**: any downstream or plugin crate implements it for its own zero-sized
/// marker type, so a novel protocol is a new family type with **zero** framework
/// `match` and zero framework release. Soundness comes from the required
/// obligation methods, checked once at registration (a token-bearer family that
/// declares a never-refresh strategy is rejected) — not from membership in a
/// closed enum. A protocol may have several families when its shapes are
/// independent (SPIFFE = an X.509-SVID family and a JWT-SVID family); a
/// cryptographically *bound* multi-shape credential (DPoP, RFC 8705) is one
/// family whose [`EGRESS`](Self::EGRESS) lists both shapes.
pub trait SchemeFamily: 'static {
    /// The wire-egress shape(s) this family presents. A slice, because
    /// sender-constrained and bound credentials present more than one at once
    /// (RFC 8705 = `[CertPresentation, InlineSecret]`).
    const EGRESS: &'static [EgressShape];

    /// The renewal strategies this family's material may legitimately use. The
    /// registration soundness check rejects a credential whose computed
    /// [`RefreshStrategy`] is not in this set.
    fn refresh_classes() -> &'static [RefreshStrategy];

    /// Cosmetic classification for UI / catalog / logging. A family with shared
    /// mechanics but a distinct display identity overrides this.
    fn pattern() -> AuthPattern;
}

/// How a credential's material is renewed as it nears or reaches expiry.
///
/// **Data, not a trait** (ADR-0088 D2). The engine reads this from the
/// credential's policy and drives the matching path; it lives here in
/// `nebula-core` (a pure data enum with no upward dependencies) so
/// [`SchemeFamily::refresh_classes`] can name it without an inverted dependency
/// edge. Re-exported from `nebula_credential` for source compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum RefreshStrategy {
    /// Valid until explicitly revoked; never auto-renewed (API key, PAT).
    #[default]
    Static,
    /// Renew without user interaction via the protocol's `refresh` — OAuth2
    /// refresh-token grant, Vault lease renew.
    RefreshToken,
    /// An external lease the engine's lease scheduler renews at a fraction of
    /// its TTL — Vault dynamic secret, Kubernetes projected token.
    Lease,
    /// Full re-acquisition round-trip; no incremental refresh — AWS STS
    /// AssumeRole, SAML/OIDC re-auth, OAuth2 without a refresh token.
    ReAcquire,
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

    /// `TestToken` exercises the manual `AuthScheme` + `SensitiveScheme`
    /// path — it derives `Zeroize`+`ZeroizeOnDrop` to satisfy
    /// `SensitiveScheme: AuthScheme + ZeroizeOnDrop`.
    #[derive(zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
    struct TestToken {
        value: String,
    }

    impl AuthScheme for TestToken {
        type Family = NoAuthFamily;
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
