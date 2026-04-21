//! OAuth2 credential — type definition, configuration, and HTTP flow.
//!
//! The [`Credential`](crate::Credential) trait implementation and the
//! `State` / `Pending` shapes live in `credential`; provider URL and
//! scope configuration in `config`; and reqwest-based HTTP flow helpers
//! (auth URL construction, token exchange, device code polling, refresh)
//! in `flow`.
//!
//! The `flow` submodule will relocate to `nebula-api` / `nebula-engine`
//! in phase P10 (see the credential architecture cleanup spec §7 and
//! ADR-0031). Until then, reqwest stays as a direct credential
//! dependency.
//!
//! # Canonical import paths
//!
//! Flat root re-exports are preferred for consumers:
//! `use nebula_credential::{OAuth2Credential, OAuth2Pending, OAuth2State};`.
//! The grant-type-specific configuration types are exposed at this
//! module path (`nebula_credential::credentials::oauth2::{OAuth2Config,
//! GrantType, …}`) — they are part of the OAuth2 surface but not hot
//! enough to deserve a crate-root alias.

mod config;
mod credential;
mod flow;

pub use config::{
    AuthCodeBuilder, AuthStyle, ClientCredentialsBuilder, DeviceCodeBuilder, GrantType,
    OAuth2Config, PkceMethod,
};
pub use credential::{OAuth2Credential, OAuth2Input, OAuth2Pending, OAuth2State};
