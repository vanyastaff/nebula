//! Built-in authentication scheme types.
//!
//! Each type implements [`AuthScheme`] and represents consumer-facing
//! auth material that resources receive. All secret fields use
//! [`SecretString`] and all `Debug` impls redact secrets.

mod api_key;
mod basic;
mod bearer;
mod database;
mod oauth2;

pub use api_key::ApiKeyAuth;
pub use basic::BasicAuth;
pub use bearer::BearerToken;
pub use database::DatabaseAuth;
pub use oauth2::OAuth2Token;
