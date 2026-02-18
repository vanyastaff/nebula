//! Tests for the Credential derive macro - successful cases.

use nebula_macros::Credential;
use serde::{Deserialize, Serialize};
include!("support.rs");

// ── Case 1: explicit input + state (no extends) ────────────────────────────

#[derive(Credential)]
#[credential(
    key = "oauth2",
    name = "OAuth 2.0",
    description = "OAuth 2.0 authentication flow",
    input = OAuth2Input,
    state = OAuth2State
)]
pub struct OAuth2Credential;

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

// ── Case 2: extends = Protocol (no explicit input/state) ───────────────────

/// Stub protocol for the trybuild test
pub struct StubApiKeyProtocol;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StubApiKeyState {
    pub server: String,
    pub token: String,
}

impl CredentialProtocol for StubApiKeyProtocol {
    type State = StubApiKeyState;

    fn parameters() -> collection::ParameterCollection {
        collection::ParameterCollection::new()
    }

    fn build_state(
        _values: &values::ParameterValues,
    ) -> Result<Self::State, core::CredentialError> {
        Ok(StubApiKeyState {
            server: "https://api.example.com".to_string(),
            token: "secret".to_string(),
        })
    }
}

#[derive(Credential)]
#[credential(
    key = "github-api",
    name = "GitHub API",
    description = "GitHub API credentials via personal access token",
    extends = StubApiKeyProtocol,
)]
pub struct GithubApi;

fn main() {
    // static description — no instance needed
    let _desc = OAuth2Credential::description();
    let _desc2 = GithubApi::description();
}
