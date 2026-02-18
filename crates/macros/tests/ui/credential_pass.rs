//! Tests for the Credential derive macro - successful cases.

use nebula_macros::Credential;
use serde::{Deserialize, Serialize};
include!("support.rs");

/// An API key credential.
#[derive(Credential)]
#[credential(
    key = "api_key",
    name = "API Key",
    description = "Simple API key authentication",
    input = ApiKeyInput,
    state = ApiKeyState
)]
pub struct ApiKeyCredential;

/// OAuth2 credential.
#[derive(Credential)]
#[credential(
    key = "oauth2",
    name = "OAuth 2.0",
    description = "OAuth 2.0 authentication flow",
    input = OAuth2Input,
    state = OAuth2State
)]
pub struct OAuth2Credential;

// Supporting types
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiKeyInput {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    pub key: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuth2Input {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: String,
}

fn main() {
    let api_key = ApiKeyCredential;
    let _ = api_key.description();

    let oauth2 = OAuth2Credential;
    let _ = oauth2.description();
}
