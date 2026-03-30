//! Built-in authentication scheme types.
//!
//! Each type implements [`AuthScheme`] and represents consumer-facing
//! auth material that resources receive. All secret fields use
//! [`SecretString`] and all `Debug` impls redact secrets.

mod api_key;
mod aws;
mod basic;
mod bearer;
mod certificate;
mod coercion;
mod database;
mod header;
mod hmac;
mod kerberos;
mod ldap;
mod oauth2;
mod saml;
mod ssh;

pub use api_key::ApiKeyAuth;
pub use aws::AwsAuth;
pub use basic::BasicAuth;
pub use bearer::BearerToken;
pub use certificate::CertificateAuth;
pub use database::{DatabaseAuth, SslMode};
pub use header::HeaderAuth;
pub use hmac::HmacSecret;
pub use kerberos::KerberosAuth;
pub use ldap::{LdapAuth, LdapBindMethod, LdapTlsMode};
pub use oauth2::OAuth2Token;
pub use saml::SamlAuth;
pub use ssh::{SshAuth, SshAuthMethod};
