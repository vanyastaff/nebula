//! Universal authentication scheme types.
//!
//! 9 built-in types cover common integration auth patterns. Plugins add
//! protocol-specific types via the open [`AuthScheme`] trait.
//!
//! The [`AuthScheme`] trait, its companion classification [`AuthPattern`], and
//! the F3 mechanics axis ([`SchemeFamily`] / [`EgressShape`]) are canonically
//! defined in `nebula_core::auth` — the cross-cutting bridge vocabulary between
//! the credential and resource systems — and re-exported through this submodule
//! for discoverability.
//!
//! **Pruned 2026-04-24** (zero consumers, Plane-A territory):
//! `FederatedAssertion` (SAML/JWT — `nebula-auth` Plane A concern per auth plane separation),
//! `OtpSeed` (TOTP/HOTP — belongs inside specific integrations, not projected),
//! `ChallengeSecret` (Digest/NTLM/SCRAM — HTTP client negotiation, not wire-level scheme).

mod auth;
mod certificate;
mod coercion;
mod connection_uri;
mod family;
mod identity_password;
mod instance_binding;
mod key_pair;
/// OAuth2 token scheme and protocol helpers (public for `scheme::oauth2::AuthStyle` path).
pub mod oauth2;
mod secret_token;
mod shared_key;
mod signing_key;

pub use auth::{
    AuthPattern, AuthScheme, EgressShape, ExternalScheme, PublicScheme, SchemeFamily,
    SensitiveScheme,
};
pub use certificate::Certificate;
pub use connection_uri::ConnectionUri;
pub use family::{
    CertificateFamily, ConnectionUriFamily, IdentityPasswordFamily, InstanceBindingFamily,
    KeyPairFamily, OAuth2Family, SecretTokenFamily, SharedKeyFamily, SigningKeyFamily,
};
pub use identity_password::IdentityPassword;
pub use instance_binding::InstanceBinding;
pub use key_pair::KeyPair;
pub use oauth2::{AuthStyle, OAuth2Token};
pub use secret_token::SecretToken;
pub use shared_key::SharedKey;
pub use signing_key::SigningKey;
