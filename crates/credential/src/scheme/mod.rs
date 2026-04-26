//! Universal authentication scheme types.
//!
//! 9 built-in types cover common integration auth patterns. Plugins add
//! protocol-specific types via the open [`AuthScheme`] trait.
//!
//! The [`AuthScheme`] trait and its companion classification [`AuthPattern`]
//! live in this submodule — they are the bridge between the credential system
//! and the resource system. Historically these two types lived in
//! `nebula-core`; they were moved here in phase P4 of the credential cleanup
//! so `nebula-core` holds only cross-cutting vocabulary.
//!
//! **Pruned 2026-04-24** (zero consumers, Plane-A territory):
//! `FederatedAssertion` (SAML/JWT — `nebula-auth` Plane A concern per ADR-0033),
//! `OtpSeed` (TOTP/HOTP — belongs inside specific integrations, not projected),
//! `ChallengeSecret` (Digest/NTLM/SCRAM — HTTP client negotiation, not wire-level scheme).

mod auth;
mod certificate;
mod coercion;
mod connection_uri;
mod identity_password;
mod instance_binding;
mod key_pair;
mod oauth2;
mod secret_token;
mod shared_key;
mod signing_key;

pub use auth::{AuthPattern, AuthScheme, PublicScheme, SensitiveScheme};
pub use certificate::Certificate;
pub use connection_uri::ConnectionUri;
pub use identity_password::IdentityPassword;
pub use instance_binding::InstanceBinding;
pub use key_pair::KeyPair;
pub use oauth2::OAuth2Token;
pub use secret_token::SecretToken;
pub use shared_key::SharedKey;
pub use signing_key::SigningKey;
