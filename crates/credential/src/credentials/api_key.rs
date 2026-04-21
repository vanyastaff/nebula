//! API Key credential -- static, non-interactive.
//!
//! The simplest credential type: a single secret token resolved from user
//! input. State and Scheme are the same type ([`SecretToken`]) via
//! [`identity_state!`](crate::identity_state).

use nebula_schema::{Field, FieldValues, HasSchema, Schema, ValidSchema};

use crate::{
    Credential, CredentialContext, NoPendingState, SecretString, error::CredentialError,
    metadata::CredentialMetadata, resolve::StaticResolveResult, scheme::SecretToken,
};

/// Typed shape of the `api_key` credential setup form.
///
/// Used solely as the [`Credential::Input`] — the value is carried through
/// the resolver as [`FieldValues`] for now; this struct exists to advertise
/// the canonical schema via [`HasSchema`].
pub struct ApiKeyInput;

impl HasSchema for ApiKeyInput {
    fn schema() -> ValidSchema {
        Schema::builder()
            .add(
                Field::string("server")
                    .label("Server URL")
                    .description("Base URL of the service (e.g. https://api.example.com)")
                    .placeholder("https://api.example.com"),
            )
            .add(
                Field::secret("api_key")
                    .label("API Key")
                    .description("Secret API token or personal access token")
                    .required(),
            )
            .build()
            .expect("api_key schema is always valid")
    }
}

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
    type Input = ApiKeyInput;
    type Scheme = SecretToken;
    type State = SecretToken;
    type Pending = NoPendingState;

    const KEY: &'static str = "api_key";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("api_key"))
            .name("API Key")
            .description("Static API key or bearer token for HTTP APIs.")
            .schema(Self::parameters())
            .pattern(nebula_core::AuthPattern::SecretToken)
            .icon("key")
            .build()
            .expect("api_key metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<SecretToken>, CredentialError> {
        let token = values.get_string_by_str("api_key").ok_or_else(|| {
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
        let original = token.token().expose_secret(ToOwned::to_owned);
        let cloned = projected.token().expose_secret(ToOwned::to_owned);
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_api_key_field() {
        let mut values = FieldValues::new();
        values.set_raw("api_key", serde_json::Value::String("sk-secret-123".into()));
        let ctx = CredentialContext::new("test-user");
        let result = ApiKeyCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(token) => {
                let exposed = token.token().expose_secret(ToOwned::to_owned);
                assert_eq!(exposed, "sk-secret-123");
            },
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_field() {
        let values = FieldValues::new();
        let ctx = CredentialContext::new("test-user");
        let result = ApiKeyCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn parameters_contains_server_and_api_key() {
        let params = ApiKeyCredential::parameters();
        assert!(params.fields().iter().any(|f| f.key().as_str() == "server"));
        assert!(
            params
                .fields()
                .iter()
                .any(|f| f.key().as_str() == "api_key")
        );
        assert_eq!(params.fields().len(), 2);
    }

    #[test]
    fn server_is_optional() {
        let params = ApiKeyCredential::parameters();
        let server = params
            .fields()
            .iter()
            .find(|f| f.key().as_str() == "server")
            .unwrap();
        assert!(!matches!(
            server.required(),
            nebula_schema::RequiredMode::Always
        ));
    }
}
