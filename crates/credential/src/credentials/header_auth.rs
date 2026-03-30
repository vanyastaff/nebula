//! Custom HTTP header authentication credential.
//!
//! A static credential (no interactive flow, no refresh) that produces
//! [`HeaderAuth`] with a configurable header name and secret value.

use nebula_parameter::values::ParameterValues;
use nebula_parameter::{Parameter, ParameterCollection};

use crate::SecretString;
use crate::core::{CredentialContext, CredentialDescription, CredentialError};
use crate::credential_trait::Credential;
use crate::pending::NoPendingState;
use crate::resolve::StaticResolveResult;
use crate::scheme::HeaderAuth;

/// Custom HTTP header credential -- resolves a header name/value pair into [`HeaderAuth`].
///
/// - **Non-interactive:** resolves in one step from user input.
/// - **Non-refreshable:** static tokens have no expiry.
/// - **Identity projection:** stored state is the scheme itself.
///
/// Used for APIs that authenticate via custom headers (e.g., `X-Api-Key`, `X-Auth-Token`).
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::credentials::HeaderAuthCredential;
/// use nebula_credential::Credential;
///
/// assert_eq!(HeaderAuthCredential::KEY, "header_auth");
/// assert!(!HeaderAuthCredential::INTERACTIVE);
/// assert!(!HeaderAuthCredential::REFRESHABLE);
/// ```
pub struct HeaderAuthCredential;

impl Credential for HeaderAuthCredential {
    type Scheme = HeaderAuth;
    type State = HeaderAuth;
    type Pending = NoPendingState;

    const KEY: &'static str = "header_auth";

    fn description() -> CredentialDescription {
        CredentialDescription {
            key: Self::KEY.to_owned(),
            name: "Custom Header Auth".to_owned(),
            description: "Custom HTTP header authentication for APIs using \
                          non-standard auth headers."
                .to_owned(),
            icon: Some("header".to_owned()),
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(
                Parameter::string("header_name")
                    .label("Header Name")
                    .description(
                        "Name of the HTTP header (e.g. X-Api-Key, X-Auth-Token)",
                    )
                    .placeholder("X-Api-Key")
                    .required(),
            )
            .add(
                Parameter::string("header_value")
                    .label("Header Value")
                    .description("Secret value for the header")
                    .required()
                    .secret(),
            )
    }

    fn project(state: &HeaderAuth) -> HeaderAuth {
        state.clone()
    }

    async fn resolve(
        values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<HeaderAuth>, CredentialError> {
        let header_name = values.get_string("header_name").ok_or_else(|| {
            CredentialError::Provider(
                "missing required field 'header_name'".to_owned(),
            )
        })?;

        let header_value = values.get_string("header_value").ok_or_else(|| {
            CredentialError::Provider(
                "missing required field 'header_value'".to_owned(),
            )
        })?;

        let secret = SecretString::new(header_value.to_owned());
        Ok(StaticResolveResult::Complete(HeaderAuth::new(
            header_name,
            secret,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_header_auth() {
        assert_eq!(HeaderAuthCredential::KEY, "header_auth");
    }

    #[test]
    fn capabilities_are_all_false() {
        assert!(!HeaderAuthCredential::INTERACTIVE);
        assert!(!HeaderAuthCredential::REFRESHABLE);
        assert!(!HeaderAuthCredential::REVOCABLE);
        assert!(!HeaderAuthCredential::TESTABLE);
    }

    #[test]
    fn project_returns_clone_of_state() {
        let auth = HeaderAuth::new("X-Api-Key", SecretString::new("test-secret"));
        let projected = HeaderAuthCredential::project(&auth);
        assert_eq!(projected.name(), auth.name());
        projected
            .value()
            .expose_secret(|v| assert_eq!(v, "test-secret"));
    }

    #[tokio::test]
    async fn resolve_creates_header_auth() {
        let mut values = ParameterValues::new();
        values.set(
            "header_name".to_owned(),
            serde_json::Value::String("X-Auth-Token".into()),
        );
        values.set(
            "header_value".to_owned(),
            serde_json::Value::String("tok_secret_123".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = HeaderAuthCredential::resolve(&values, &ctx)
            .await
            .unwrap();
        match result {
            StaticResolveResult::Complete(header_auth) => {
                assert_eq!(header_auth.name(), "X-Auth-Token");
                header_auth
                    .value()
                    .expose_secret(|v| assert_eq!(v, "tok_secret_123"));
            }
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_fails_without_header_name() {
        let mut values = ParameterValues::new();
        values.set(
            "header_value".to_owned(),
            serde_json::Value::String("tok_123".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = HeaderAuthCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_fails_without_header_value() {
        let mut values = ParameterValues::new();
        values.set(
            "header_name".to_owned(),
            serde_json::Value::String("X-Api-Key".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = HeaderAuthCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn parameters_contains_header_name_and_value() {
        let params = HeaderAuthCredential::parameters();
        assert!(params.contains("header_name"));
        assert!(params.contains("header_value"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn header_name_is_required() {
        let params = HeaderAuthCredential::parameters();
        let name = params.get("header_name").unwrap();
        assert!(name.required);
    }

    #[test]
    fn header_value_is_required() {
        let params = HeaderAuthCredential::parameters();
        let value = params.get("header_value").unwrap();
        assert!(value.required);
    }
}
