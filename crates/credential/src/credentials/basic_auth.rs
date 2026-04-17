//! HTTP Basic Auth credential -- static, non-interactive.
//!
//! Resolves a username + password pair into [`IdentityPassword`]. State and
//! Scheme are the same type via [`identity_state!`](crate::identity_state).

use nebula_parameter::{Parameter, ParameterCollection, values::ParameterValues};

use crate::{
    SecretString, context::CredentialContext, credential::Credential, error::CredentialError,
    metadata::CredentialMetadata, pending::NoPendingState, resolve::StaticResolveResult,
    scheme::IdentityPassword,
};

/// HTTP Basic Auth credential -- resolves username + password into
/// [`IdentityPassword`].
///
/// - **Non-interactive:** resolves in one step from user input.
/// - **Non-refreshable:** static credentials have no expiry.
/// - **Identity projection:** stored state is the scheme itself.
pub struct BasicAuthCredential;

impl Credential for BasicAuthCredential {
    type Scheme = IdentityPassword;
    type State = IdentityPassword;
    type Pending = NoPendingState;

    const KEY: &'static str = "basic_auth";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata {
            key: Self::KEY.to_owned(),
            name: "Basic Auth".to_owned(),
            description: "HTTP Basic authentication (username + password).".to_owned(),
            icon: Some("lock".to_owned()),
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
            pattern: nebula_core::AuthPattern::IdentityPassword,
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(
                Parameter::string("username")
                    .label("Username")
                    .description("Username for HTTP Basic authentication")
                    .required(),
            )
            .add(
                Parameter::string("password")
                    .label("Password")
                    .description("Password for HTTP Basic authentication")
                    .required()
                    .secret(),
            )
    }

    fn project(state: &IdentityPassword) -> IdentityPassword {
        state.clone()
    }

    async fn resolve(
        values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<IdentityPassword>, CredentialError> {
        let username = values.get_string("username").ok_or_else(|| {
            CredentialError::Provider("missing required field 'username'".to_owned())
        })?;
        let password = values.get_string("password").ok_or_else(|| {
            CredentialError::Provider("missing required field 'password'".to_owned())
        })?;
        let secret = SecretString::new(password.to_owned());
        Ok(StaticResolveResult::Complete(IdentityPassword::new(
            username, secret,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_basic_auth() {
        assert_eq!(BasicAuthCredential::KEY, "basic_auth");
    }

    #[test]
    fn capabilities_are_all_false() {
        const { assert!(!BasicAuthCredential::INTERACTIVE) };
        const { assert!(!BasicAuthCredential::REFRESHABLE) };
        const { assert!(!BasicAuthCredential::REVOCABLE) };
        const { assert!(!BasicAuthCredential::TESTABLE) };
    }

    #[test]
    fn project_returns_clone_of_state() {
        let auth = IdentityPassword::new("admin", SecretString::new("s3cret"));
        let projected = BasicAuthCredential::project(&auth);
        assert_eq!(projected.identity(), "admin");
        let original = auth.password().expose_secret(|s| s.to_owned());
        let cloned = projected.password().expose_secret(|s| s.to_owned());
        assert_eq!(original, cloned);
    }

    #[tokio::test]
    async fn resolve_extracts_username_and_password() {
        let mut values = ParameterValues::new();
        values.set(
            "username".to_owned(),
            serde_json::Value::String("alice".into()),
        );
        values.set(
            "password".to_owned(),
            serde_json::Value::String("p@ssw0rd".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = BasicAuthCredential::resolve(&values, &ctx).await.unwrap();
        match result {
            StaticResolveResult::Complete(auth) => {
                assert_eq!(auth.identity(), "alice");
                let pw = auth.password().expose_secret(|s| s.to_owned());
                assert_eq!(pw, "p@ssw0rd");
            },
            _ => panic!("expected Complete variant"),
        }
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_username() {
        let mut values = ParameterValues::new();
        values.set(
            "password".to_owned(),
            serde_json::Value::String("secret".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = BasicAuthCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn resolve_returns_error_on_missing_password() {
        let mut values = ParameterValues::new();
        values.set(
            "username".to_owned(),
            serde_json::Value::String("alice".into()),
        );
        let ctx = CredentialContext::new("test-user");
        let result = BasicAuthCredential::resolve(&values, &ctx).await;
        assert!(result.is_err());
    }
}
