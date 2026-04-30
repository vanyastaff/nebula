//! Built-in credential type implementations.
//!
//! Each type implements [`Credential`](crate::Credential) using
//! the v2 unified trait. Static credentials (API key, basic auth) use
//! [`identity_state!`](crate::identity_state) so that `State = Scheme`.
//!
//! Per Phase 5 of the M6 dependency redesign each credential ships a
//! `<Name>Properties` companion struct (`#[derive(Schema, Deserialize)]`)
//! that owns the setup-form schema; `Credential::Properties` points at
//! it and the engine reads the schema via
//! `Credential::properties_schema()`.

pub mod api_key;
pub mod basic_auth;
pub mod oauth2;
pub mod oauth2_config;

pub use api_key::{ApiKeyCredential, ApiKeyProperties};
pub use basic_auth::{BasicAuthCredential, BasicAuthProperties};
pub use oauth2::{OAuth2Credential, OAuth2Pending, OAuth2Properties, OAuth2State};
pub use oauth2_config::{
    AuthCodeBuilder, AuthStyle, ClientCredentialsBuilder, DeviceCodeBuilder, GrantType,
    OAuth2Config, PkceMethod,
};
