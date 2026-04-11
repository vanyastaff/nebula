//! API Key credential -- static, non-interactive.
//!
//! The simplest credential type: a single secret token resolved from user
//! input. State and Scheme are the same type ([`SecretToken`]) via
//! [`identity_state!`](crate::identity_state).

use nebula_parameter::{Parameter, ParameterCollection, values::ParameterValues};

use crate::{
    SecretString, context::CredentialContext, credential::Credential,
    description::CredentialDescription, error::CredentialError, pending::NoPendingState,
    resolve::StaticResolveResult, scheme::SecretToken,
};

/// API Key credential -- resolves a single token into a [`SecretToken`].
///
/// - **Non-interactive:** resolves in one step from user input.
/// - **Non-refreshable:** static tokens have no expiry.
/// - **Identity projection:** stored state is the scheme itself.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::credentials::ApiKeyCredential;
/// use nebula_credential::Credential;
///
/// assert_eq!(ApiKeyCredential::KEY, "api_key");
/// assert!(!ApiKeyCredential::INTERACTIVE);
/// assert!(!ApiKeyCredential::REFRESHABLE);
/// ```
pub struct ApiKeyCredential;

impl Credential for ApiKeyCredential {
    type Scheme = SecretToken;
    type State = SecretToken;
    type Pending = NoPendingState;

    const KEY: &'static str = "api_key";

    fn description() -> CredentialDescription {
        CredentialDescription {
            key: Self::KEY.to_owned(),
            name: "API Key".to_owned(),
            description: "Static API key or bearer token for HTTP APIs.".to_owned(),
            icon: Some("key".to_owned()),
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
            pattern: nebula_core::AuthPattern::SecretToken,
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(
                Parameter::string("server")
                    .label("Server URL")
                    .description("Base URL of the service (e.g. https://api.example.com)")
                    .placeholder("https://api.example.com"),
            )
            .add(
                Parameter::string("api_key")
                    .label("API Key")
                    .description("Secret API token or personal access token")
                    .required()
                    .secret(),
            )
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        let token = values.get_string("api_key").ok_or_else(|| {
            CredentialError::Provider("missing required field 'api_key'".to_owned())
        })?;
        let secret = SecretString::new(token.to_owned());
        Ok(StaticResolveResult::Complete(SecretToken::new(secret)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_api_key() {
        assert_eq!(ApiKeyCredential::KEY, "api_key");
    }

    #[test]
    fn capabilities_are_all_false() {
        const { assert!(!ApiKeyCredential::INTERACTIVE) };
        const { assert!(!ApiKeyCredential::REFRESHABLE) };
        const { assert!(!ApiKeyCredential::REVOCABLE) };
        const { assert!(!ApiKeyCredential::TESTABLE) };
    }

    #[test]
    fn project_returns_clone_of_state() {
        let token = SecretToken::new(SecretString::new("test-token"));
        let projected = ApiKeyCredential::project(&token);
        let original = token.token().expose_secret(|s| s.to_owned());
        let cloned = projected.token().expose_secret(|s| s.to_owned());
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_api_key_field() {
        let mut values = ParameterValues::new();
        values.set(
            "api_key".to_owned(),
            serde_json::Value::String("sk-secret-123".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = ApiKeyCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(token) => {
                let exposed = token.token().expose_secret(|s| s.to_owned());
                assert_eq!(exposed, "sk-secret-123");
            }
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_field() {
        let values = ParameterValues::new();
        let ctx = CredentialContext::new("test-user");
        let result = ApiKeyCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn parameters_contains_server_and_api_key() {
        let params = ApiKeyCredential::parameters();
        assert!(params.contains("server"));
        assert!(params.contains("api_key"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn server_is_optional() {
        let params = ApiKeyCredential::parameters();
        let server = params.get("server").unwrap();
        assert!(!server.required);
    }
}
