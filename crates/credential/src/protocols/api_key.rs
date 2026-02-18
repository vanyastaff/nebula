//! ApiKey protocol — reusable `server` + `token` credential block.

use serde::{Deserialize, Serialize};

use nebula_parameter::collection::ParameterCollection;
use nebula_parameter::def::ParameterDef;
use nebula_parameter::types::{SecretParameter, TextParameter};
use nebula_parameter::values::ParameterValues;

use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::CredentialProtocol;

/// State produced by [`ApiKeyProtocol`] after initialization.
///
/// Accessible in nodes via `ctx.credential::<MyApi>().await?`.
/// The `token` field is kept as a plain `String` here — zeroization
/// is handled at the storage layer via `scrub_ephemeral`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyState {
    /// Base URL of the service (e.g. `https://api.github.com`)
    pub server: String,
    /// Secret API token / personal access token
    pub token: String,
}

impl ApiKeyState {
    /// Expose the token for use in HTTP headers.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Expose the server URL.
    pub fn server(&self) -> &str {
        &self.server
    }
}

impl CredentialState for ApiKeyState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "api_key";

    fn scrub_ephemeral(&mut self) {
        // Overwrite token in memory before persistence if desired.
        // Currently a no-op — token must be stored for later use.
    }
}

/// Protocol that contributes `server` + `token` fields.
///
/// Used as a base for any service authenticated via a bearer/API token.
///
/// # Usage
///
/// ```ignore
/// #[derive(Credential)]
/// #[credential(
///     key = "slack-api",
///     name = "Slack API",
///     extends = ApiKeyProtocol,
/// )]
/// pub struct SlackApi;
/// ```
pub struct ApiKeyProtocol;

impl CredentialProtocol for ApiKeyProtocol {
    type State = ApiKeyState;

    fn parameters() -> ParameterCollection {
        let mut server = TextParameter::new("server", "Server URL");
        server.metadata.description =
            Some("Base URL of the service (e.g. https://api.github.com)".into());
        server.metadata.required = true;

        let mut token = SecretParameter::new("token", "API Token");
        token.metadata.description = Some("Secret API token or personal access token".into());
        token.metadata.required = true;

        ParameterCollection::new()
            .with(ParameterDef::Text(server))
            .with(ParameterDef::Secret(token))
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let server = values
            .get_string("server")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: server".into()),
            })?
            .to_owned();

        let token = values
            .get_string("token")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: token".into()),
            })?
            .to_owned();

        Ok(ApiKeyState { server, token })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parameters_contains_server_and_token() {
        let params = ApiKeyProtocol::parameters();
        assert!(params.contains("server"));
        assert!(params.contains("token"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn server_is_required() {
        let params = ApiKeyProtocol::parameters();
        assert!(params.get_by_key("server").unwrap().is_required());
    }

    #[test]
    fn token_is_secret_and_required() {
        let params = ApiKeyProtocol::parameters();
        let token = params.get_by_key("token").unwrap();
        assert!(token.is_required());
        assert!(matches!(token, ParameterDef::Secret(_)));
    }

    #[test]
    fn build_state_produces_correct_state() {
        let mut values = ParameterValues::new();
        values.set("server", json!("https://api.github.com"));
        values.set("token", json!("ghp_secret123"));

        let state = ApiKeyProtocol::build_state(&values).unwrap();
        assert_eq!(state.server(), "https://api.github.com");
        assert_eq!(state.token(), "ghp_secret123");
    }

    #[test]
    fn build_state_missing_server_returns_error() {
        let mut values = ParameterValues::new();
        values.set("token", json!("ghp_secret123"));

        assert!(ApiKeyProtocol::build_state(&values).is_err());
    }

    #[test]
    fn build_state_missing_token_returns_error() {
        let mut values = ParameterValues::new();
        values.set("server", json!("https://api.github.com"));

        assert!(ApiKeyProtocol::build_state(&values).is_err());
    }
}
