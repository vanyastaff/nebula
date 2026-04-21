//! OAuth2 credential — type definition, configuration, and HTTP flow.
//!
//! The [`Credential`](crate::Credential) trait implementation and the
//! `State` / `Pending` shapes live in `credential`; provider URL and
//! scope configuration in `config`; **authorization URL construction**
//! (no HTTP client) in [`authorize_url`]; and reqwest-based token
//! exchange, device code polling, and refresh in `flow` when the
//! **`oauth2-http`** feature is enabled (default).
//!
//! Disabling `oauth2-http` removes the `reqwest` dependency so the crate
//! can type-check in slim dependency graphs; interactive flows that need
//! token exchange return [`CredentialError::Provider`] with a message
//! pointing at the feature flag (ADR-0031 incremental split).
//!
//! # Canonical import paths
//!
//! Flat root re-exports are preferred for consumers:
//! `use nebula_credential::{OAuth2Credential, OAuth2Pending, OAuth2State};`.
//! The grant-type-specific configuration types are exposed at this
//! module path (`nebula_credential::credentials::oauth2::{OAuth2Config,
//! GrantType, …}`) — they are part of the OAuth2 surface but not hot
//! enough to deserve a crate-root alias.

mod authorize_url;
mod config;
mod credential;
#[cfg(feature = "oauth2-http")]
mod flow;

pub use config::{
    AuthCodeBuilder, AuthStyle, ClientCredentialsBuilder, DeviceCodeBuilder, GrantType,
    OAuth2Config, PkceMethod,
};
pub use credential::{OAuth2Credential, OAuth2Input, OAuth2Pending, OAuth2State};
