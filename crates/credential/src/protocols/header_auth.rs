//! HeaderAuth protocol — arbitrary header name + secret value.

use serde::{Deserialize, Serialize};

use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Field, Schema};

use crate::core::{CredentialError, CredentialState, ValidationError};
use crate::traits::StaticProtocol;

/// State produced by [`HeaderAuthProtocol`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderAuthState {
    pub header_name: String,
    pub header_value: String,
}

impl CredentialState for HeaderAuthState {
    const VERSION: u16 = 1;
    const KIND: &'static str = "header_auth";

    fn scrub_ephemeral(&mut self) {}
}

/// Protocol that contributes `header_name` + `header_value` fields.
///
/// Used as a base for any service authenticated via a custom HTTP header.
pub struct HeaderAuthProtocol;

impl StaticProtocol for HeaderAuthProtocol {
    type State = HeaderAuthState;

    fn parameters() -> Schema {
        Schema::new()
            .field(
                Field::text("header_name")
                    .with_label("Header Name")
                    .with_placeholder("X-Auth-Token")
                    .required(),
            )
            .field(
                Field::text("header_value")
                    .with_label("Header Value")
                    .required()
                    .secret(),
            )
    }

    fn build_state(values: &ParameterValues) -> Result<Self::State, CredentialError> {
        let header_name = values
            .get_string("header_name")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat(
                    "missing required field: header_name".into(),
                ),
            })?
            .to_owned();

        let header_value = values
            .get_string("header_value")
            .ok_or_else(|| CredentialError::Validation {
                source: ValidationError::InvalidFormat(
                    "missing required field: header_value".into(),
                ),
            })?
            .to_owned();

        Ok(HeaderAuthState {
            header_name,
            header_value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parameters_has_header_name_and_value() {
        let params = HeaderAuthProtocol::parameters();
        assert!(params.contains("header_name"));
        assert!(params.contains("header_value"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn build_state_produces_state() {
        let mut values = ParameterValues::new();
        values.set("header_name", json!("X-Auth-Token"));
        values.set("header_value", json!("tok_123"));
        let state = HeaderAuthProtocol::build_state(&values).unwrap();
        assert_eq!(state.header_name, "X-Auth-Token");
        assert_eq!(state.header_value, "tok_123");
    }

    #[test]
    fn missing_header_name_returns_error() {
        let mut values = ParameterValues::new();
        values.set("header_value", json!("tok_123"));
        assert!(HeaderAuthProtocol::build_state(&values).is_err());
    }
}
