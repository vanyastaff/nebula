//! Universal authentication scheme types.
//!
//! 12 built-in types cover common auth patterns. Plugins add protocol-specific
//! types via the open [`AuthScheme`](nebula_core::AuthScheme) trait.

mod certificate;
mod challenge_secret;
mod coercion;
mod connection_uri;
mod federated_assertion;
mod identity_password;
mod instance_binding;
mod key_pair;
mod oauth2;
mod otp_seed;
mod secret_token;
mod shared_key;
mod signing_key;

pub use certificate::Certificate;
pub use challenge_secret::ChallengeSecret;
pub use connection_uri::ConnectionUri;
pub use federated_assertion::FederatedAssertion;
pub use identity_password::IdentityPassword;
pub use instance_binding::InstanceBinding;
pub use key_pair::KeyPair;
pub use oauth2::OAuth2Token;
pub use otp_seed::OtpSeed;
pub use secret_token::SecretToken;
pub use shared_key::SharedKey;
pub use signing_key::SigningKey;
