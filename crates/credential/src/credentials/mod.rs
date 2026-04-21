//! Built-in credential type implementations.
//!
//! Each type implements [`Credential`](crate::Credential) using
//! the v2 unified trait. Static credentials (API key, basic auth) use
//! [`identity_state!`](crate::identity_state) so that `State = Scheme`.

pub mod api_key;
pub mod basic_auth;
pub mod oauth2;

pub use api_key::ApiKeyCredential;
pub use basic_auth::BasicAuthCredential;
pub use oauth2::{OAuth2Credential, OAuth2Pending, OAuth2State};
