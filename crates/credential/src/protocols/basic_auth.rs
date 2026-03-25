//! BasicAuth protocol — username + password credential block.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};

use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Parameter, ParameterCollection};

use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::StaticProtocol;

/// State produced by [`BasicAuthProtocol`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicAuthState {
    pub username: String,
    pub password: String,
}

impl BasicAuthState {
    /// Base64-encoded `username:password` for `Authorization: Basic` header.
    #[must_use]
    pub fn encoded(&self) -> String {
        BASE64.encode(format!("{}:{}", self.username, self.password))
    }
}

impl CredentialState for BasicAuthState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "basic_auth";

    fn scrub_ephemeral(&mut self) {}
}

/// Protocol that contributes `username` + `password` fields.
///
/// Used as a base for any service authenticated via HTTP Basic Auth.
pub struct BasicAuthProtocol;

impl StaticProtocol for BasicAuthProtocol {
    type State = BasicAuthState;

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(Parameter::string("username").label("Username").required())
            .add(
                Parameter::string("password")
                    .label("Password")
                    .required()
                    .secret(),
            )
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let username = values
            .get_string("username")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: username".into()),
            })?
            .to_owned();

        let password = values
            .get_string("password")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat("missing required field: password".into()),
            })?
            .to_owned();

        Ok(BasicAuthState { username, password })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parameters_has_username_and_password() {
        let params = BasicAuthProtocol::parameters();
        assert!(params.contains("username"));
        assert!(params.contains("password"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn build_state_produces_state() {
        let mut values = ParameterValues::new();
        values.set("username", json!("alice"));
        values.set("password", json!("s3cr3t"));
        let state = BasicAuthProtocol::build_state(&values).unwrap();
        assert_eq!(state.username, "alice");
        assert_eq!(state.password, "s3cr3t");
    }

    #[test]
    fn encoded_produces_correct_base64() {
        let state = BasicAuthState {
            username: "alice".into(),
            password: "s3cr3t".into(),
        };
        // base64("alice:s3cr3t") = "YWxpY2U6czNjcjN0"
        assert_eq!(state.encoded(), "YWxpY2U6czNjcjN0");
    }

    #[test]
    fn missing_username_returns_error() {
        let mut values = ParameterValues::new();
        values.set("password", json!("s3cr3t"));
        assert!(BasicAuthProtocol::build_state(&values).is_err());
    }
}
