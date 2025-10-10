//! `OAuth2` flows

pub mod authorization_code;
pub mod client_credentials;
pub mod common;

pub use authorization_code::{
    AuthorizationCodeFlow, AuthorizationCodeInput, OAuth2AuthorizationCode,
};
pub use client_credentials::{
    ClientCredentialsFlow, ClientCredentialsInput, OAuth2ClientCredentials,
};
pub use common::{OAuth2State, TokenResponse, oauth2_refresh_token};
